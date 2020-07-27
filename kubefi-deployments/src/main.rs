extern crate dotenv;
extern crate env_logger;
extern crate kube_derive;
extern crate kube_runtime;
extern crate kubefi_deployments;
#[macro_use]
extern crate log;

use std::fs;
use std::path::Path;

use anyhow::{Error, Result};
use dotenv::dotenv;
use futures::{StreamExt, TryStreamExt};
use k8s_openapi::apiextensions_apiserver::pkg::apis::apiextensions::v1beta1::CustomResourceDefinition;
use kube::api::Meta;
use kube::api::{Api, ListParams, PostParams};
use kube::Client;
use kube_runtime::watcher::Event;
use tokio::time::{delay_for, Duration};

use kubefi_deployments::controller::{NiFiController, ReplaceStatus};
use kubefi_deployments::crd::{create_new_version, delete_old_version, NiFiDeployment};
use kubefi_deployments::operator_config::read_config;
use kubefi_deployments::{get_api, read_namespace};

#[tokio::main]
async fn main() -> Result<()> {
    dotenv().ok();
    env_logger::init();

    let client = Client::try_default().await?;

    // Manage CRDs first
    let crds: Api<CustomResourceDefinition> = Api::all(client.clone());

    delete_old_version(crds.clone()).await?;
    delay_for(Duration::from_secs(2)).await;

    let schema = fs::read_to_string("conf/schema.json")?;
    create_new_version(crds, schema).await?;
    delay_for(Duration::from_secs(1)).await;

    let namespace = read_namespace();
    let api: Api<NiFiDeployment> = get_api(&namespace, client.clone());

    let mut watcher = kube_runtime::watcher(api.clone(), ListParams::default()).boxed();
    let config = read_config()?;
    debug!("Loaded config {}", config);
    let controller =
        NiFiController::new(namespace, client.clone(), config, Path::new("./templates"))?;

    info!(
        "Starting Kubefi event loop for {:?}",
        std::any::type_name::<NiFiDeployment>()
            .split("::")
            .last()
            .unwrap()
    );

    while let Some(event) = watcher.try_next().await? {
        let status = handle_event(&controller, event.clone()).await?;
        match status {
            Some(s) => {
                let api: Api<NiFiDeployment> = Api::namespaced(client.clone(), s.ns.as_str());
                replace_status(&api, s).await
            }
            None => Ok(()),
        }?;
    }

    Err(Error::msg(
        "Event stream for NiFiDeployment was closed, exiting...".to_string(),
    ))
}

async fn replace_status(api: &Api<NiFiDeployment>, s: ReplaceStatus) -> Result<()> {
    debug!("patching status: {:?}", &s);
    let mut resource = api.get_status(&s.name).await?;
    resource.status = Some(s.clone().status);
    let pp = PostParams::default();
    let data = serde_json::to_vec(&resource)?;
    match api.replace_status(&s.name, &pp, data).await {
        Ok(_) => {
            info!("Status updated: {:?}", s.status);
            Ok(())
        }
        Err(e) => {
            error!("Update status failed {}", e);
            Ok(())
        }
    }
}

async fn handle_event(
    controller: &NiFiController,
    event: Event<NiFiDeployment>,
) -> Result<Option<ReplaceStatus>> {
    match event {
        Event::Applied(o) => {
            let spec = o.spec.clone();
            info!("applied deployment: {} (spec={:?})", Meta::name(&o), spec);
            controller.on_add(o).await
        }
        Event::Restarted(o) => {
            let length = o.len();
            info!("Got Restarted event with length: {}", length);
            Ok(None)
        }
        Event::Deleted(o) => {
            info!("deleting Deployment: {}", Meta::name(&o));
            controller.on_delete(o).await.map(|_| None)
        }
    }
}
