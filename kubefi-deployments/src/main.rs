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
use kube::api::Api;
use kube::api::Meta;
use kube::api::WatchEvent;
use kube::Client;
use kube::runtime::Informer;
use tokio::time::{delay_for, Duration};

use kubefi_deployments::controller::NiFiController;
use kubefi_deployments::crd::{create_new_version, delete_old_version, NiFiDeployment, NiFiDeploymentStatus};
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
    let api_deployments: Api<NiFiDeployment> = get_api(&namespace, client.clone());

    let informer = Informer::new(api_deployments);
    let defaults = read_config();
    let controller = NiFiController::new(namespace, client, defaults,
                                         Path::new("./templates"))?;

    info!("Starting Kubefi event loop for {:?}",
          std::any::type_name::<NiFiDeployment>().split("::").last().unwrap());

    let mut stream = informer.poll().await?.boxed();
    while let Some(event) = stream.try_next().await? {
        handle(&controller, event).await; //TODO: handle status
    }
    Ok(())
}

fn read_config() -> Config {
    let image_name = env::var("IMAGE_NAME").ok();
    let storage_class = env::var("STORAGE_CLASS").ok();
    Config { image_name, storage_class }
}

fn read_namespace() -> Namespace {
    let ns = std::env::var("NAMESPACE").unwrap_or("default".into());
    match ns.as_str() {
        "all" => Namespace::All,
        _ => Namespace::SingleNamespace(ns)
    }
}

async fn handle(controller: &NiFiController, event: WatchEvent<NiFiDeployment>) -> Result<Option<NiFiDeploymentStatus>> {
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
            Ok(Some(NiFiDeploymentStatus { last_action: e.message }))
        }
        _ => Ok(None)
    }
}
