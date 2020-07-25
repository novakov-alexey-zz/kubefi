extern crate anyhow;
extern crate kube;
extern crate kube_derive;
extern crate serde;

use std::path::Path;
use std::{error, fmt};

use anyhow::Error;
use k8s_openapi::api::apps::v1::StatefulSet;
use k8s_openapi::api::core::v1::{ConfigMap, Service};
use k8s_openapi::api::extensions::v1beta1::Ingress;
use k8s_openapi::Resource;
use kube::api::{Meta, PostParams};
use kube::{Api, Client};
use serde::de::DeserializeOwned;
use serde::Serialize;
use serde_json::Value;

use crate::anyhow::Result;
use crate::controller::ControllerError::MissingProperty;
use crate::crd::{NiFiDeployment, NiFiDeploymentSpec, NiFiDeploymentStatus};
use crate::template::Template;
use crate::Namespace;

#[derive(Debug)]
pub enum ControllerError {
    MissingProperty(String, String),
    MissingTemplateParameter(String),
}

pub struct ReplaceStatus {
    pub name: String,
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
        let error = self
            .handle_event(d, &name)
            .await
            .err()
            .map(|e| e.to_string())
            .unwrap_or_default();
        let status = NiFiDeploymentStatus { error, last_action };
        Ok(Some(ReplaceStatus { name, status }))
    }

    pub async fn on_modify(&self, d: NiFiDeployment) -> Result<Option<ReplaceStatus>> {
        self.handle_action(d, "modify".to_string()).await
    }

    pub async fn on_delete(&self, _: NiFiDeployment) -> Result<()> {
        Ok(())
    }

    fn get_api<T: Resource>(&self, ns: &str) -> Api<T> {
        Api::namespaced(self.client.clone(), &ns)
    }

    async fn handle_event(&self, d: NiFiDeployment, name: &str) -> Result<()> {
        let ns = d
            .clone()
            .metadata
            .namespace
            .ok_or_else(|| MissingProperty("namespace".to_string(), d.kind.clone()))?;

        let zk_cm_name = format!("{}-zookeeper", &name);
        let zk_cm = self.create_from_yaml::<ConfigMap, _>(&zk_cm_name, &name, &ns, |name: &str| {
            self.template.zk_configmap(name)
        });

        let nifi_cm_name = format!("{}-config", &name);
        let nifi_cm =
            self.create_from_yaml::<ConfigMap, _>(&nifi_cm_name, &name, &ns, |name: &str| {
                self.template.nifi_configmap(name)
            });

        let (r1, r2) = futures::future::join(zk_cm, nifi_cm).await;
        r1.and(r2)?;

        let statefulsets: Api<StatefulSet> = self.get_api(&ns);
        let nifi = self.create_nifi_set(&d.spec, &name, &statefulsets);
        let zk = self.create_zk_set(&d.spec, &name, &statefulsets);
        let (r1, r2) = futures::future::join(nifi, zk).await;
        r1.and(r2)?;

        let service = self.create_from_yaml::<Service, _>(&name, &name, &ns, |name| {
            self.template.nifi_service(name)
        });

        let headless_service_name = format!("{}-headless", &name);
        let headless_service =
            self.create_from_yaml::<Service, _>(&headless_service_name, &name, &ns, |name| {
                self.template.nifi_headless_service(name)
            });

        let zk_service_name = format!("{}-zookeeper", &name);
        let zk_service =
            self.create_from_yaml::<Service, _>(&zk_service_name, &name, &ns, |name| {
                self.template.zk_service(name)
            });

        let zk_headless_service_name = format!("{}-zookeeper-headless", &name);
        let zk_headless_service =
            self.create_from_yaml::<Service, _>(&zk_headless_service_name, &name, &ns, |name| {
                self.template.zk_headless_service(name)
            });

        let ingress_name = format!("{}-ingress", &name);
        let ingress = self.create_from_yaml::<Ingress, _>(&ingress_name, &name, &ns, |name| {
            self.template.ingress(name)
        });

        let (r1, r2, r3, r4, r5) = futures::future::join5(
            service,
            headless_service,
            zk_service,
            zk_headless_service,
            ingress,
        )
        .await;
        r1.and(r2).and(r3).and(r4).and(r5)?;

        Ok(())
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
        let api: Api<T> = self.get_api(&ns);
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

    async fn create_nifi_set(
        &self,
        d: &NiFiDeploymentSpec,
        name: &str,
        api: &Api<StatefulSet>,
    ) -> Result<Option<StatefulSet>> {
        match api.get(&name).await {
            Err(_) => {
                let nifi_set = self.nifi_set_resource(&d, &name)?;
                match nifi_set {
                    Some(s) => self.create_resource(&api, s).await.map(Some),
                    None => Ok(None),
                }
            }
            Ok(s) => Ok(Some(s)),
        }
    }

    async fn create_zk_set(
        &self,
        spec: &NiFiDeploymentSpec,
        name: &str,
        api: &Api<StatefulSet>,
    ) -> Result<Option<StatefulSet>> {
        match api.get(&name).await {
            Err(_) => {
                let zk_set = self.zk_set_resource(&spec, &name)?;
                match zk_set {
                    Some(s) => self.create_resource(&api, s).await.map(Some),
                    None => Ok(None),
                }
            }
            Ok(s) => Ok(Some(s)),
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

    fn nifi_set_resource(
        &self,
        spec: &NiFiDeploymentSpec,
        deployment_name: &str,
    ) -> Result<Option<StatefulSet>> {
        let image_name = &spec.image_name;
        let storage_class = &spec.storage_class;
        let yaml = self.template.nifi_statefulset(
            deployment_name,
            &spec.nifi_replicas,
            &image_name,
            &storage_class,
        )?;
        yaml.iter()
            .fold(Ok(None), |_, y| Ok(Some(NiFiController::from_yaml(&y)?)))
    }

    fn zk_set_resource(
        &self,
        spec: &NiFiDeploymentSpec,
        deployment_name: &str,
    ) -> Result<Option<StatefulSet>> {
        let image_name = &spec.zk_image_name;
        let storage_class = &spec.storage_class;
        let yaml = self.template.zk_statefulset(
            deployment_name,
            &spec.zk_replicas,
            &image_name,
            &storage_class,
        )?;
        yaml.iter()
            .fold(Ok(None), |_, y| Ok(Some(NiFiController::from_yaml(&y)?)))
    }

    fn from_yaml<T: Resource + DeserializeOwned>(yaml: &str) -> Result<T, Error> {
        serde_yaml::from_str(&yaml).map_err(Error::new)
    }
}
