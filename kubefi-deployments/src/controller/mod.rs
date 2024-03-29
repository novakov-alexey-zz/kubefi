extern crate anyhow;
extern crate either;
extern crate kube;
extern crate kube_derive;
extern crate serde;

use std::fmt::Debug;
use std::rc::Rc;
use std::{error, fmt};

use anyhow::Error;
use k8s_openapi::api::apps::v1::StatefulSet;
use k8s_openapi::api::core::v1::{ConfigMap, Service};
use k8s_openapi::api::extensions::v1beta1::Ingress;
use k8s_openapi::Resource;
use kube::api::{DeleteParams, ListParams, Meta, PostParams};
use kube::{Api, Client};
use serde::de::DeserializeOwned;
use serde::Serialize;

use crate::anyhow::Result;
use crate::controller::configmap::ConfigMapController;
use crate::controller::service::ServiceController;
use crate::controller::statefulset::StatefulSetController;
use crate::controller::ControllerError::MissingProperty;
use crate::crd::{NiFiDeployment, NiFiDeploymentStatus};
use crate::template::Template;
use crate::{read_type, Namespace};

use self::either::Either;
use self::either::Either::{Left, Right};

mod configmap;
mod service;
mod statefulset;

const KUBEFI_LABELS: &str = "app.kubernetes.io/managed-by=Kubefi,release=nifi";
const NIFI_APP_LABEL: &str = "nifi";
const ZK_APP_LABEL: &str = "zookeeper";

#[derive(Debug)]
pub enum ControllerError {
    MissingProperty(String, String),
}

#[derive(Serialize, Debug, Clone)]
pub struct ReplaceStatus {
    pub name: String,
    pub ns: String,
    pub status: NiFiDeploymentStatus,
}

impl fmt::Display for ControllerError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            ControllerError::MissingProperty(property, kind) => write!(
                f,
                "Property {:?} for {} resource is missing",
                property, kind
            ),
        }
    }
}

impl error::Error for ControllerError {
    fn source(&self) -> Option<&(dyn error::Error + 'static)> {
        match *self {
            ControllerError::MissingProperty(_, _) => None,
        }
    }
}

pub struct NiFiController {
    pub namespace: Namespace,
    client: Rc<Client>,
    cm_controller: ConfigMapController,
    svc_controller: ServiceController,
    sets_controller: StatefulSetController,
}

#[derive(Clone, Debug)]
pub struct ConfigMapState {
    pub updated: bool,
    pub logging_cm: Option<String>,
}

impl NiFiController {
    pub fn new(
        ns: Namespace,
        client: Rc<Client>,
        template: Rc<Template>,
    ) -> Result<NiFiController> {
        let cm_controller = ConfigMapController {
            client: client.clone(),
            template: template.clone(),
        };
        let svc_controller = ServiceController {
            client: client.clone(),
            template: template.clone(),
        };
        let sets_controller = StatefulSetController {
            client: client.clone(),
            template,
        };
        Ok(NiFiController {
            namespace: ns,
            client,
            cm_controller,
            svc_controller,
            sets_controller,
        })
    }

    pub async fn on_apply(&self, d: NiFiDeployment) -> Result<Option<ReplaceStatus>> {
        let name = read_name(&d)?;
        let ns = read_namespace(&d)?;
        let status = match self.handle_event(d.clone(), &name, &ns).await {
            Ok(true) => {
                let status = NiFiDeploymentStatus {
                    nifi_replicas: d.spec.nifi_replicas,
                    error_msg: "".to_string(),
                };
                Some(ReplaceStatus { name, ns, status })
            }
            Ok(_) => None,
            Err(e) => {
                let status = NiFiDeploymentStatus {
                    nifi_replicas: d.spec.nifi_replicas,
                    error_msg: e.to_string(),
                };
                Some(ReplaceStatus { name, ns, status })
            }
        };
        Ok(status)
    }

    pub async fn on_delete(&self, d: NiFiDeployment) -> Result<()> {
        let ns = read_namespace(&d)?;
        let params = &DeleteParams::default();
        let lp = ListParams::default().labels(KUBEFI_LABELS);

        let sts = self.delete_resources::<StatefulSet>(&ns, &params, &lp);
        let svc = self.delete_resources::<Service>(&ns, &params, &lp);
        let cm = self.delete_resources::<ConfigMap>(&ns, &params, &lp);
        let ing = self.delete_resources::<Ingress>(&ns, &params, &lp);
        let (r1, r2, r3, r4) = futures::future::join4(sts, svc, cm, ing).await;
        r1.and(r2).and(r3).and(r4)
    }

    async fn delete_resources<T: Resource + Clone + DeserializeOwned + Meta + Debug>(
        &self,
        ns: &str,
        params: &DeleteParams,
        lp: &ListParams,
    ) -> Result<()> {
        let names = find_names::<T>(&self.client, &ns, &lp).await?;
        debug!("{} to delete: {:?}", read_type::<T>("Resources"), &names);
        let api = get_api::<T>(&self.client, &ns);
        let deletes = names.iter().map(|name| api.delete(&name, &params));
        futures::future::join_all(deletes)
            .await
            .into_iter()
            .map(|r| {
                r.map(|e| {
                    e.map_left(|resource| debug!("Deleted {}", Meta::name(&resource)))
                        .map_right(|status| debug!("Deleting {:?}", status))
                })
                .map(|_| ())
            })
            .fold(Ok(()), |acc, r| acc.and(r.map_err(Error::from)))
    }

    async fn handle_event(&self, d: NiFiDeployment, name: &str, ns: &str) -> Result<bool> {
        let nifi_cm_updated = self.cm_controller.handle_configmaps(&d, &name, &ns).await?;
        let cm_state = ConfigMapState {
            updated: nifi_cm_updated,
            logging_cm: d.clone().spec.logging_config_map,
        };
        let service_updated = self
            .svc_controller
            .handle_services(&name, &ns, &d.spec.ingress)
            .await?;
        let sets_updated = self
            .sets_controller
            .handle_sets(&d, &name, &ns, cm_state, service_updated)
            .await?;
        debug!(
            "Resource updates: configmap = {}, statefulsets = {}, services = {}",
            nifi_cm_updated, sets_updated, service_updated
        );
        Ok(nifi_cm_updated || sets_updated || service_updated)
    }
}

fn read_name(d: &NiFiDeployment) -> Result<String> {
    d.clone()
        .metadata
        .name
        .ok_or_else(|| Error::from(MissingProperty("name".to_string(), d.kind.clone())))
}

fn read_namespace(d: &NiFiDeployment) -> Result<String, Error> {
    d.clone()
        .metadata
        .namespace
        .ok_or_else(|| Error::from(MissingProperty("namespace".to_string(), d.kind.clone())))
}

async fn get_or_create<
    T: Resource + Serialize + Clone + DeserializeOwned + Meta,
    F: FnOnce(&str) -> Result<Option<String>>,
>(
    client: &Client,
    name: &str,
    cr_name: &str,
    ns: &str,
    get_yaml: F,
) -> Result<Either<Option<T>, Option<T>>> {
    get_or_create_convert(client, name, cr_name, ns, get_yaml, Ok).await
}

async fn get_or_create_convert<
    T: Resource + Serialize + Clone + DeserializeOwned + Meta,
    F: FnOnce(&str) -> Result<Option<String>>,
    C: FnOnce(T) -> Result<T>,
>(
    client: &Client,
    name: &str,
    cr_name: &str,
    ns: &str,
    get_yaml: F,
    convert: C,
) -> Result<Either<Option<T>, Option<T>>> {
    let api = get_api::<T>(&client.clone(), &ns);
    match api.get(&name).await {
        Err(_) => create_from_yaml(&cr_name, &ns, &client, get_yaml, convert).await,
        Ok(res) => {
            debug!("Found existing {}: {}", read_type::<T>("resource"), &name);
            Ok(Left(Some(res)))
        }
    }
}

async fn create_from_yaml<
    T: Resource + Serialize + Clone + DeserializeOwned + Meta,
    F: FnOnce(&str) -> Result<Option<String>>,
    C: FnOnce(T) -> Result<T>,
>(
    cr_name: &str,
    ns: &str,
    client: &Client,
    get_yaml: F,
    convert: C,
) -> Result<Either<Option<T>, Option<T>>, Error> {
    let yaml = get_yaml(&cr_name)?;
    match yaml {
        Some(y) => {
            let resource = from_yaml(&y)?;
            let converted = convert(resource)?;
            let api = get_api::<T>(&client.clone(), &ns);
            create_resource(&api, converted).await.map(Some).map(Right)
        }
        None => {
            debug!(
                "{} template for {} is not enabled or missing ",
                read_type::<T>("resource"),
                cr_name
            );
            Ok(Right(None))
        }
    }
}

async fn create_resource<T: Serialize + Clone + DeserializeOwned + Meta>(
    api: &Api<T>,
    resource: T,
) -> Result<T> {
    let pp = PostParams::default();
    api.create(&pp, &resource).await.map_err(Error::new)
}

fn from_yaml<T: Resource + Serialize + Clone + DeserializeOwned + Meta>(
    y: &str,
) -> Result<T, Error> {
    serde_yaml::from_str(&y).map_err(Error::new)
}

async fn delete_resources<T: Resource + Clone + DeserializeOwned + Meta + Debug>(
    client: &Client,
    ns: &str,
    params: &DeleteParams,
    lp: &ListParams,
) -> Result<()> {
    let names = find_names::<T>(&client, &ns, &lp).await?;
    debug!("{} to delete: {:?}", read_type::<T>("Resources"), &names);
    let api = get_api::<T>(&client, &ns);
    let deletes = names.iter().map(|name| api.delete(&name, &params));
    futures::future::join_all(deletes)
        .await
        .into_iter()
        .map(|r| {
            r.map(|e| {
                e.map_left(|resource| debug!("Deleted {}", Meta::name(&resource)))
                    .map_right(|status| debug!("Deleting {:?}", status))
            })
            .map(|_| ())
        })
        .fold(Ok(()), |acc, r| acc.and(r.map_err(Error::from)))
}

async fn find_names<T: Resource + Clone + DeserializeOwned + Meta>(
    client: &Client,
    ns: &str,
    lp: &ListParams,
) -> Result<Vec<String>> {
    let api: Api<T> = get_api(&client, &ns);
    let list = &api.list(&lp).await?;
    let names = list.into_iter().map(Meta::name).collect();
    Ok(names)
}

fn get_api<T: Resource>(client: &Client, ns: &str) -> Api<T> {
    Api::namespaced(client.clone(), &ns)
}
