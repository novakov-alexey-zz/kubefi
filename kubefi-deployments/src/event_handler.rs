use anyhow::Result;
use kube::api::{Meta, PostParams};
use kube::Api;
use kube_runtime::watcher::Event;

use crate::controller::{NiFiController, ReplaceStatus};
use crate::crd::NiFiDeployment;

pub async fn replace_status(api: &Api<NiFiDeployment>, s: ReplaceStatus) -> Result<()> {
    debug!("replacing status: {:?}", &s);
    let mut resource = api.get_status(&s.name).await?;
    resource.status = Some(s.clone().status);
    let pp = PostParams::default();
    let data = serde_json::to_vec(&resource)?;
    api.replace_status(&s.name, &pp, data)
        .await
        .map(|_| {
            info!("Status updated: {:?}", s.status);
            Ok(())
        })
        .unwrap_or_else(|e| {
            error!("Update status failed {}", e);
            Ok(())
        })
}

pub async fn handle_event(
    controller: &NiFiController,
    event: Event<NiFiDeployment>,
) -> Result<Vec<ReplaceStatus>> {
    match event {
        Event::Applied(event) => {
            let spec = event.spec.clone();
            info!(
                "applied deployment: {} (spec={:?})",
                Meta::name(&event),
                spec
            );
            controller
                .on_apply(event)
                .await
                .map(|status| status.into_iter().collect())
        }
        Event::Restarted(events) => {
            let length = events.len();
            info!("Got Restarted event with length: {}", length);
            let applies = events.into_iter().map(|e| controller.on_apply(e));
            futures::future::join_all(applies)
                .await
                .into_iter()
                .fold(Ok(Vec::new()), |acc, res| {
                    acc.and_then(|mut all_res: Vec<ReplaceStatus>| {
                        res.map(|r| {
                            let mut l = r.into_iter().collect::<Vec<_>>();
                            all_res.append(&mut l);
                            all_res
                        })
                    })
                })
        }
        Event::Deleted(event) => {
            info!("deleting Deployment: {}", Meta::name(&event));
            controller.on_delete(event).await.map(|_| Vec::new())
        }
    }
}
