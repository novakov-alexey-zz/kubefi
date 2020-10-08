extern crate dotenv;
extern crate env_logger;
extern crate kube_derive;
extern crate kube_runtime;
extern crate kubefi_deployments;
#[macro_use]
extern crate log;

use std::fs;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use anyhow::{Error, Result};
use dotenv::dotenv;
use futures::{StreamExt, TryStreamExt};
use k8s_openapi::apiextensions_apiserver::pkg::apis::apiextensions::v1beta1::CustomResourceDefinition;
use kube::api::Meta;
use kube::api::{Api, ListParams, PostParams};
use kube::Client;
use kube_runtime::watcher::Event;
use tokio::time::{delay_for, Duration};

use kubefi_deployments::config::{read_kubefi_config, read_nifi_config};
use kubefi_deployments::controller::{NiFiController, ReplaceStatus};
use kubefi_deployments::crd::{create_new_version, delete_old_version, NiFiDeployment};
use kubefi_deployments::template::Template;
use kubefi_deployments::{get_api, read_namespace, read_type, Namespace};

#[tokio::main]
async fn main() -> Result<()> {
    dotenv().ok();
    env_logger::init();
    let version = env!("CARGO_PKG_VERSION");
    let banner = r#"
     _  __     _           __ _
    | |/ /    | |         / _(_)
    | ' /_   _| |__   ___| |_ _
    |  <| | | | '_ \ / _ \  _| |
    | . \ |_| | |_) |  __/ | | |
    |_|\_\__,_|_.__/ \___|_| |_|
    "#;
    println!("{}\nversion: {}\n", banner, version);

    let kubefi_cfg = read_kubefi_config()?;
    debug!(">>>> Loaded Kubefi config {:?}", kubefi_cfg);
    let client = Client::try_default().await?;

    let crds: Api<CustomResourceDefinition> = Api::all(client.clone());
    if kubefi_cfg.replace_existing_crd {
        replace_crd(crds, kubefi_cfg.crd_schema_path).await?;
    }

    let namespace = read_namespace();
    let api = get_api::<NiFiDeployment>(&namespace, client.clone());

    let mut watcher = kube_runtime::watcher(api.clone(), ListParams::default()).boxed();
    let nifi_cfg = read_nifi_config()?;
    debug!(">>>> Loaded NiFi config {}", &nifi_cfg);

    let controller = NiFiController::new(
        namespace,
        Rc::new(client.clone()),
        Rc::new(Template::new(Path::new("./templates"), nifi_cfg)?),
    )?;

    info!(
        "Starting Kubefi event loop for {:?}",
        read_type::<NiFiDeployment>("NiFi")
    );

    while let Some(event) = watcher.try_next().await? {
        let status = handle_event(&controller, event.clone()).await?;
        for s in status {
            let api = get_api::<NiFiDeployment>(
                &Namespace::SingleNamespace(s.ns.as_str().to_string()),
                client.clone(),
            );
            replace_status(&api, s).await?
        }
    }

    Err(Error::msg(format!(
        "Event stream for {:?} was closed, exiting...",
        read_type::<NiFiDeployment>("NiFi")
    )))
}

async fn replace_crd(crds: Api<CustomResourceDefinition>, schema: PathBuf) -> Result<()> {
    delete_old_version(crds.clone()).await?;
    delay_for(Duration::from_secs(2)).await;

    let schema = fs::read_to_string(schema)?;
    create_new_version(crds, schema).await?;
    delay_for(Duration::from_secs(1)).await;
    Ok(())
}

async fn replace_status(api: &Api<NiFiDeployment>, s: ReplaceStatus) -> Result<()> {
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

async fn handle_event(
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
