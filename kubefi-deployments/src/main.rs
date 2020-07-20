extern crate dotenv;
extern crate env_logger;
extern crate kube_derive;
extern crate kubefi_deployments;
#[macro_use]
extern crate log;

use std::env;
use std::fs;
use std::path::Path;

use anyhow::Result;
use dotenv::dotenv;
use futures::{StreamExt, TryStreamExt};
use k8s_openapi::apiextensions_apiserver::pkg::apis::apiextensions::v1::CustomResourceDefinition;
use kube::api::{Api, PostParams};
use kube::api::Meta;
use kube::api::WatchEvent;
use kube::Client;
use kube::runtime::Informer;
use tokio::time::{delay_for, Duration};

use kubefi_deployments::controller::{NiFiController, ReplaceStatus};
use kubefi_deployments::crd::{create_new_version, delete_old_version, NiFiDeployment};
use kubefi_deployments::get_api;
use kubefi_deployments::Namespace;
use kubefi_deployments::operator_config::Config;

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

    let informer = Informer::new(api.clone());
    let defaults = read_config();
    let controller = NiFiController::new(namespace, client, defaults,
                                         Path::new("./templates"))?;

    info!("Starting Kubefi event loop for {:?}",
          std::any::type_name::<NiFiDeployment>().split("::").last().unwrap());

    let mut stream = informer.poll().await?.boxed();
    while let Some(event) = stream.try_next().await? {
        let status = handle_event(&controller, event.clone()).await?;
        match status {
            Some(s) => replace_status(&api, s).await,
            None => Ok(())
        }?;
    }
    Ok(())
}

async fn replace_status(api: &Api<NiFiDeployment>, s: ReplaceStatus) -> Result<()> {
    let mut resource = api.get_status(&s.name).await?;
    resource.status = Some(s.status);
    let pp = PostParams::default();
    let data = serde_json::to_vec(&resource)?;
    match api.replace_status(s.name.as_str(), &pp, data).await.map(|_| ()) {
        Ok(_) => {
            info!("Status updated");
            Ok(())
        }
        Err(e) => {
            error!("Update status failed {:?}", e);
            Ok(())
        }
    }
}

fn read_config() -> Config {
    let image = env::var("IMAGE_NAME").ok();
    let zk_image = env::var("ZK_IMAGE_NAME").ok();
    let storage_class = env::var("STORAGE_CLASS").ok();
    Config { image, zk_image, storage_class }
}

fn read_namespace() -> Namespace {
    let ns = std::env::var("NAMESPACE").unwrap_or("default".into());
    match ns.as_str() {
        "all" => Namespace::All,
        _ => Namespace::SingleNamespace(ns)
    }
}

async fn handle_event(controller: &NiFiController, event: WatchEvent<NiFiDeployment>) -> Result<Option<ReplaceStatus>> {
    match event {
        WatchEvent::Added(o) => {
            let spec = o.spec.clone();
            println!("added deployment: {} (spec={:?})", Meta::name(&o), spec);
            controller.on_add(o).await
        }
        WatchEvent::Modified(o) => {
            let status = o.status.clone().unwrap();
            println!(
                "modified Deployment: {} (status={:?})",
                Meta::name(&o),
                status
            );
            controller.on_modify(o).await
        }
        WatchEvent::Deleted(o) => {
            println!("deleted Deployment: {}", Meta::name(&o));
            controller.on_delete(o).await.map(|_| None)
        }
        WatchEvent::Error(e) => {
            println!("Error event: {:?}", e);
            //let status1 = NiFiDeploymentStatus { error: e.message, last_action: "error".to_string() };
            Ok(None)
        }
        _ => Ok(None)
    }
}
