extern crate anyhow;
extern crate kube;
extern crate kube_derive;
extern crate serde;

use std::{error, fmt};
use std::path::Path;

use anyhow::Error;
use k8s_openapi::api::apps::v1::StatefulSet;
use kube::{Api, Client};
use kube::api::PostParams;

use crate::anyhow::Result;
use crate::controller::ControllerError::{MissingProperty, MissingTemplateParameter};
use crate::crd::{NiFiDeployment, NiFiDeploymentStatus};
use crate::Namespace;
use crate::operator_config::Config;
use crate::template::Template;

#[derive(Debug)]
pub enum ControllerError {
    MissingProperty(String, String),
    MissingTemplateParameter(String),
}

impl fmt::Display for ControllerError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            ControllerError::MissingProperty(property, kind) =>
                write!(f, "Property {:?} for {} resource is missing", property, kind),
            ControllerError::MissingTemplateParameter(parameter) =>
                write!(f, "Template parameter {:?} is not specificed in the resource nor in controller config", parameter)
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

    pub async fn on_add(&self, d: NiFiDeployment) -> Result<Option<NiFiDeploymentStatus>> {
        self.handle_event(d).await?;
        Ok(Some(NiFiDeploymentStatus { last_action: "added".to_string() }))
    }

    pub async fn on_modify(&self, _: NiFiDeployment) -> Result<Option<NiFiDeploymentStatus>> {
        Ok(Some(NiFiDeploymentStatus { last_action: "modified".to_string() }))
    }

    pub async fn on_delete(&self, _: NiFiDeployment) -> Result<()> {
        Ok(())
    }

    async fn handle_event(&self, d: NiFiDeployment) -> Result<()> {
        let name = d.clone().metadata.name.ok_or(MissingProperty("name".to_string(), d.kind.clone()))?;
        let statefulsets: Api<StatefulSet> = super::get_api(&self.namespace, self.client.clone());
        let statefulset = statefulsets.get(&name).await;
        match statefulset {
            Err(_) => {
                let nifi_set = self.statefulset_template(&d, &name)?;
                self.create_statefulset(d.clone(), statefulsets, nifi_set).await
            }
            _ => Ok(())
        }?;


        Ok(())
    }

    async fn create_statefulset(&self, d: NiFiDeployment, api: Api<StatefulSet>, set: StatefulSet) -> Result<()> {
        let pp = PostParams::default();
        api.create(&pp, &set).await
            .map(|_| ()).map_err(|e| Error::from(e))
    }

    fn statefulset_template(&self, d: &NiFiDeployment, deployment_name: &String) -> Result<StatefulSet> {
        let replicas = d.spec.nifi_replicas;
        let image_name = d.spec.image_name.clone().or(self.defaults.image_name.clone())
            .ok_or(MissingTemplateParameter("image_name".to_string()))?;
        let storage_class = d.spec.storage_class.clone().or(self.defaults.storage_class.clone())
            .ok_or(MissingTemplateParameter("storage_class".to_string()))?;
        let yaml = self.template.nifi_statefulset_for(deployment_name, &replicas,
                                                      &image_name, &storage_class)?;
        unimplemented!()
    }
}