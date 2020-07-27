extern crate anyhow;
extern crate kube;
extern crate kube_derive;
extern crate serde;

use std::fmt::Debug;
use std::path::Path;
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
use serde_json::Value;

use crate::anyhow::Result;
use crate::controller::ControllerError::MissingProperty;
use crate::crd::{NiFiDeployment, NiFiDeploymentStatus};
use crate::template::Template;
use crate::Namespace;

#[derive(Debug)]
pub enum ControllerError {
    MissingProperty(String, String),
    MissingTemplateParameter(String),
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
            ControllerError::MissingProperty(property, kind) =>
                write!(f, "Property {:?} for {} resource is missing", property, kind),
            ControllerError::MissingTemplateParameter(parameter) =>
                write!(f,
                       "Template parameter {:?} is not specified in the resource nor in Kubefi-deployment controller config",
                       parameter)
        }
    }
}

impl error::Error for ControllerError {
    fn source(&self) -> Option<&(dyn error::Error + 'static)> {
        match *self {
            ControllerError::MissingProperty(_, _) => None,
            ControllerError::MissingTemplateParameter(_) => None,
        }
    }
}

pub struct NiFiController {
    pub namespace: Namespace,
    pub client: Client,
    template: Template,
}

impl NiFiController {
    pub fn new(ns: Namespace, client: Client, config: Value, template_path: &Path) -> Result<Self> {
        let template = Template::new(template_path, config)?;
        Ok(NiFiController {
            namespace: ns,
            client,
            template,
        })
    }

    pub async fn on_add(&self, d: NiFiDeployment) -> Result<Option<ReplaceStatus>> {
        self.handle_action(d, "add".to_string()).await
    }

    async fn handle_action(
        &self,
        d: NiFiDeployment,
        last_action: String,
    ) -> Result<Option<ReplaceStatus>, Error> {
        let name = d
            .clone()
            .metadata
            .name
            .ok_or_else(|| MissingProperty("name".to_string(), d.kind.clone()))?;
        let ns = NiFiController::read_namespace(&d)?;
        let error = self
            .handle_event(d, &name, &ns)
            .await
            .err()
            .map(|e| e.to_string())
            .unwrap_or_default();
        let status = NiFiDeploymentStatus {
            error_msg: error,
            last_action,
        };
        Ok(Some(ReplaceStatus { name, ns, status }))
    }

    pub async fn on_delete(&self, d: NiFiDeployment) -> Result<()> {
        let ns = NiFiController::read_namespace(&d)?;
        let params = &DeleteParams::default();
        let lp = ListParams::default().labels("app.kubernetes.io/managed-by=Kubefi,release=nifi");

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
        let api = self.get_api::<T>(&ns);
        let names = self.find_names::<T>(&ns, &lp).await?;
        debug!("Resources to delete: {:?}", &names);
        let deletes = names.iter().map(|name| api.delete(&name, &params));
        futures::future::join_all(deletes)
            .await
            .into_iter()
            .map(|r| {
                r.map(|e| {
                    e.map_left(|resource| debug!("Deleted {:?}", resource))
                        .map_right(|status| debug!("Deleting {:?}", status))
                })
                .map(|_| ())
            })
            .fold(Ok(()), |acc, r| acc.and(r.map_err(Error::from)))
    }

    async fn find_names<T: Resource + Clone + DeserializeOwned + Meta>(
        &self,
        ns: &str,
        lp: &ListParams,
    ) -> Result<Vec<String>> {
        let api: Api<T> = self.get_api(&ns);
        let list = &api.list(&lp).await?;
        let names = list.into_iter().map(Meta::name).collect::<Vec<String>>();
        Ok(names)
    }

    fn get_api<T: Resource>(&self, ns: &str) -> Api<T> {
        Api::namespaced(self.client.clone(), &ns)
    }

    async fn handle_event(&self, d: NiFiDeployment, name: &str, ns: &str) -> Result<()> {
        let zk_cm_name = format!("{}-zookeeper", &name);
        let zk_cm = self.create_from_yaml::<ConfigMap, _>(&zk_cm_name, &name, &ns, |name| {
            self.template.zk_configmap(name)
        });

        let nifi_cm_name = format!("{}-config", &name);
        let nifi_cm = self.create_from_yaml::<ConfigMap, _>(&nifi_cm_name, &name, &ns, |name| {
            self.template.nifi_configmap(name)
        });

        let (r1, r2) = futures::future::join(zk_cm, nifi_cm).await;
        r1.and(r2)?;

        let nifi = self.create_from_yaml::<StatefulSet, _>(&name, &name, &ns, |name| {
            let image_name = &d.spec.image_name;
            let storage_class = &d.spec.storage_class;
            self.template.nifi_statefulset(
                &name,
                &d.spec.nifi_replicas,
                &image_name,
                &storage_class,
            )
        });
        let zk_set_name = NiFiController::zk_set_name(&name);
        let zk = self.create_from_yaml::<StatefulSet, _>(&zk_set_name, &name, &ns, |name| {
            let image_name = &d.spec.zk_image_name;
            let storage_class = &d.spec.storage_class;
            self.template
                .zk_statefulset(&name, &d.spec.zk_replicas, &image_name, &storage_class)
        });
        let (r1, r2) = futures::future::join(nifi, zk).await;
        r1.and(r2)?;

        let svc = self.create_from_yaml::<Service, _>(&name, &name, &ns, |name| {
            self.template.nifi_service(name)
        });

        let headless_svc_name = format!("{}-headless", &name);
        let headless_svc =
            self.create_from_yaml::<Service, _>(&headless_svc_name, &name, &ns, |name| {
                self.template.nifi_headless_service(name)
            });

        let zk_svc_name = format!("{}-zookeeper", &name);
        let zk_svc = self.create_from_yaml::<Service, _>(&zk_svc_name, &name, &ns, |name| {
            self.template.zk_service(name)
        });

        let zk_headless_svc_name = format!("{}-zookeeper-headless", &name);
        let zk_headless_svc =
            self.create_from_yaml::<Service, _>(&zk_headless_svc_name, &name, &ns, |name| {
                self.template.zk_headless_service(name)
            });

        let ingress_name = format!("{}-ingress", &name);
        let ingress = self.create_from_yaml::<Ingress, _>(&ingress_name, &name, &ns, |name| {
            self.template.ingress(name)
        });

        let (r1, r2, r3, r4, r5) =
            futures::future::join5(svc, headless_svc, zk_svc, zk_headless_svc, ingress).await;
        r1.and(r2).and(r3).and(r4).and(r5)?;

        Ok(())
    }

    fn zk_set_name(name: &str) -> String {
        format!("{}-zookeeper", &name)
    }

    fn read_namespace(d: &NiFiDeployment) -> Result<String, Error> {
        d.clone()
            .metadata
            .namespace
            .ok_or_else(|| Error::from(MissingProperty("namespace".to_string(), d.kind.clone())))
    }

    async fn create_from_yaml<
        T: Resource + Serialize + Clone + DeserializeOwned + Meta,
        F: FnOnce(&str) -> Result<Option<String>>,
    >(
        &self,
        name: &str,
        cr_name: &str,
        ns: &str,
        yaml: F,
    ) -> Result<Option<T>> {
        let api = self.get_api::<T>(&ns);
        match api.get(&name).await {
            Err(_) => {
                let yaml = yaml(&cr_name)?;
                match yaml {
                    Some(y) => {
                        let resource = NiFiController::from_yaml(&y)?;
                        self.create_resource(&api, resource).await.map(Some)
                    }
                    None => Ok(None),
                }
            }
            Ok(cm) => Ok(Some(cm)),
        }
    }

    async fn create_resource<T: Serialize + Clone + DeserializeOwned + Meta>(
        &self,
        api: &Api<T>,
        resource: T,
    ) -> Result<T> {
        let pp = PostParams::default();
        api.create(&pp, &resource).await.map_err(Error::new)
    }

    fn from_yaml<T: Resource + DeserializeOwned>(yaml: &str) -> Result<T, Error> {
        serde_yaml::from_str(&yaml).map_err(Error::new)
    }
}
