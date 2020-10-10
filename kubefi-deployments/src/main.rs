extern crate dotenv;
extern crate env_logger;
extern crate futures_core;
extern crate kube_derive;
extern crate kube_runtime;
extern crate kubefi_deployments;
#[macro_use]
extern crate log;

use std::path::Path;
use std::rc::Rc;

use anyhow::Result;
use dotenv::dotenv;
use futures::StreamExt;
use k8s_openapi::apiextensions_apiserver::pkg::apis::apiextensions::v1beta1::CustomResourceDefinition;
use kube::api::{Api, ListParams};
use kube::Client;

use kubefi_deployments::config::{read_kubefi_config, read_nifi_config};
use kubefi_deployments::controller::NiFiController;
use kubefi_deployments::crd::{replace_crd, NiFiDeployment};
use kubefi_deployments::template::Template;
use kubefi_deployments::watcher::watch;
use kubefi_deployments::{get_api, read_namespace, read_type};

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

    watch(client, &mut watcher, &controller).await
}
