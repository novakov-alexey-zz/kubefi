use std::collections::BTreeMap;
use std::path::Path;

use anyhow::{Error, Result};
use handlebars::Handlebars;

pub struct Template {
    handlebars: Handlebars<'static>,
}

const NIFI_STATEFULSET: &str = "nifi-statefulset";
const ZK_STATEFULSET: &str = "zk-statefulset";
const SERVICE: &str = "service";
const INGRESS: &str = "ingress";
const CONFIGMAP: &str = "configmap";
const TEMPLATE_FILE_EXTENSION: &str = ".yaml";

impl Template {
    pub fn new(path: &Path) -> Result<Self> {
        let mut handlebars = Handlebars::new();
        handlebars.register_templates_directory(TEMPLATE_FILE_EXTENSION, path)?;
        Ok(Template { handlebars })
    }

    pub fn nifi_statefulset(&self, name: &String, replicas: &u8, image_name: &String,
                            storage_class: &String) -> Result<String> {
        self.statefulset(name, replicas, image_name, storage_class, NIFI_STATEFULSET)
    }

    pub fn zk_statefulset(&self, name: &String, replicas: &u8, image_name: &String,
                          storage_class: &String) -> Result<String> {
        self.statefulset(name, replicas, image_name, storage_class, ZK_STATEFULSET)
    }

    pub fn service(&self, name: &String) -> Result<String> {
        let data: BTreeMap<&str, &String> = [("name", name)].iter().cloned().collect();
        self.render(&data, SERVICE)
    }

    pub fn ingress(&self, name: &String, ingress_class: &String, host: &String) -> Result<String> {
        let data: BTreeMap<&str, &String> = [
            ("name", name),
            ("ingressClass", ingress_class),
            ("host", host)]
            .iter().cloned().collect();
        self.render(&data, INGRESS)
    }

    fn render(&self, data: &BTreeMap<&str, &String>, template: &str) -> Result<String> {
        self.handlebars.render(template, &data)
            .map_err(|e| Error::new(e))
    }

    fn statefulset(&self, name: &String, replicas: &u8, image_name: &String,
                   storage_class: &String, template: &str) -> Result<String> {
        let replicas_str = &replicas.to_string();
        let data: BTreeMap<&str, &String> = [
            ("name", name),
            ("imageName", image_name),
            ("replicas", replicas_str),
            ("storageClass", storage_class)]
            .iter().cloned().collect();
        self.render(&data, template)
    }
}