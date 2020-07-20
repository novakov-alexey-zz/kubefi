extern crate anyhow;
extern crate kube;
extern crate kube_derive;
extern crate serde;

use std::{error, fmt};
use std::path::Path;

use anyhow::Error;
use k8s_openapi::api::apps::v1::StatefulSet;
use k8s_openapi::api::core::v1::Service;
use k8s_openapi::api::extensions::v1beta1::Ingress;
use k8s_openapi::Resource;
use kube::{Api, Client};
use kube::api::{Meta, PostParams};
use serde::de::DeserializeOwned;
use serde::Serialize;

use crate::anyhow::Result;
use crate::controller::ControllerError::{MissingProperty, MissingTemplateParameter};
use crate::crd::{NiFiDeployment, NiFiDeploymentSpec, NiFiDeploymentStatus};
use crate::Namespace;
use crate::operator_config::{Config, IngressConfig};
use crate::template::Template;

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
    pub defaults: Config,
    template: Template,
}

impl NiFiController {
    pub fn new(ns: Namespace, client: Client, defaults: Config, template_path: &Path) -> Result<Self> {
        let template = Template::new(template_path)?;
        Ok(NiFiController { namespace: ns, client, defaults, template })
    }

    pub async fn on_add(&self, d: NiFiDeployment) -> Result<Option<ReplaceStatus>> {
        self.handle_action(d, "add".to_string()).await
    }

    async fn handle_action(&self, d: NiFiDeployment, last_action: String) -> Result<Option<ReplaceStatus>, Error> {
        let name = d.clone().metadata.name.ok_or(MissingProperty("name".to_string(), d.kind.clone()))?;
        let error = self.handle_event(d, &name).await
            .err().map(|e| e.to_string()).unwrap_or(String::new());
        let status = NiFiDeploymentStatus { error, last_action };
        Ok(Some(ReplaceStatus { name, status }))
    }

    pub async fn on_modify(&self, d: NiFiDeployment) -> Result<Option<ReplaceStatus>> {
        self.handle_action(d, "modify".to_string()).await
    }

    pub async fn on_delete(&self, _: NiFiDeployment) -> Result<()> {
        Ok(())
    }

    async fn handle_event(&self, d: NiFiDeployment, name: &String) -> Result<()> {
        let statefulsets: Api<StatefulSet> = super::get_api(&self.namespace, self.client.clone());

        // StatefulSets
        let nifi = self.create_nifi_set(&d.spec, &name, &statefulsets);
        let zk = self.create_zk_set(&d.spec, &name, &statefulsets);
        let (r1, r2) = futures::future::join(nifi, zk).await;
        r1.or(r2)?;

        // Service
        self.create_service(&name).await?;
        // Ingress
        if let Some(ingress) = &self.defaults.ingress {
            self.create_ingress(&name, &ingress).await?;
        }


        Ok(())
    }

    async fn create_ingress(&self, name: &String, ingress: &IngressConfig) -> Result<Ingress> {
        let api: Api<Ingress> = super::get_api(&self.namespace, self.client.clone());
        match api.get(&name).await {
            Err(_) => {
                let yaml = self.template.ingress(&name, &ingress.ingress_class, &ingress.host)?;
                let nifi_ingress = NiFiController::from_yaml(&yaml)?;
                self.create_resource(&api, nifi_ingress).await
            }
            Ok(s) => Ok(s)
        }
    }

    async fn create_service(&self, name: &String) -> Result<Service> {
        let api: Api<Service> = super::get_api(&self.namespace, self.client.clone());
        match api.get(&name).await {
            Err(_) => {
                let yaml = self.template.service(&name)?;
                let nifi_service = NiFiController::from_yaml(&yaml)?;
                self.create_resource(&api, nifi_service).await
            }
            Ok(s) => Ok(s)
        }
    }

    async fn create_nifi_set(&self, d: &NiFiDeploymentSpec, name: &String, api: &Api<StatefulSet>)
                             -> Result<StatefulSet> {
        match api.get(&name).await {
            Err(_) => {
                let nifi_set = self.nifi_set_resource(&d, &name)?;
                self.create_resource(api, nifi_set).await
            }
            ok => ok.map_err(|e| Error::new(e))
        }
    }

    async fn create_zk_set(&self, spec: &NiFiDeploymentSpec, name: &String, api: &Api<StatefulSet>)
                           -> Result<StatefulSet> {
        match api.get(&name).await {
            Err(_) => {
                let zk_set = self.zk_set_resource(&spec, &name)?;
                self.create_resource(api, zk_set).await
            }
            ok => ok.map_err(|e| Error::new(e))
        }
    }

    async fn create_resource<T: Serialize + Clone + DeserializeOwned + Meta>(
        &self, api: &Api<T>, resource: T) -> Result<T> {
        let pp = PostParams::default();
        api.create(&pp, &resource).await.map_err(|e| Error::new(e))
    }

    fn nifi_set_resource(&self, spec: &NiFiDeploymentSpec, deployment_name: &String) -> Result<StatefulSet> {
        let image_name = spec.image_name.clone().or(self.defaults.image.clone())
            .ok_or(MissingTemplateParameter("image_name".to_string()))?;
        let storage_class = self.read_storage_class(spec)?;
        let yaml = self.template
            .nifi_statefulset(deployment_name, &spec.nifi_replicas, &image_name, &storage_class)?;
        NiFiController::from_yaml(&yaml)
    }

    fn zk_set_resource(&self, spec: &NiFiDeploymentSpec, deployment_name: &String) -> Result<StatefulSet> {
        let image_name = spec.zk_image_name.clone().or(self.defaults.zk_image.clone())
            .ok_or(MissingTemplateParameter("zk_image_name".to_string()))?;
        let storage_class = self.read_storage_class(spec)?;
        let yaml = self.template
            .zk_statefulset(deployment_name, &spec.zk_replicas, &image_name, &storage_class)?;
        NiFiController::from_yaml(&yaml)
    }

    fn read_storage_class(&self, spec: &NiFiDeploymentSpec) -> Result<String, ControllerError> {
        spec.storage_class.clone().or(self.defaults.storage_class.clone())
            .ok_or(MissingTemplateParameter("storage_class".to_string()))
    }

    fn from_yaml<T: Resource + DeserializeOwned>(yaml: &String) -> Result<T, Error> {
        serde_yaml::from_str(&yaml).map_err(|e| Error::new(e))
    }
}