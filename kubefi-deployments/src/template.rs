use std::path::Path;

use anyhow::{Error, Result};
use handlebars::Handlebars;
use serde_json::Value;

use crate::handelbars_ext::get_files_helper;

pub struct Template {
    handlebars: Handlebars<'static>,
    config: Value,
}

const NIFI_STATEFULSET: &str = "nifi-statefulset";
const NIFI_SERVICE: &str = "nifi-service";
const NIFI_HEADLESS_SERVICE: &str = "nifi-headless-service";
const NIFI_CONFIGMAP: &str = "nifi-configmap";
const INGRESS: &str = "ingress";

const ZK_STATEFULSET: &str = "zk-statefulset";
const ZK_SERVICE: &str = "zk-service";
const ZK_HEADLESS_SERVICE: &str = "zk-headless-service";
const ZK_CONFIGMAP: &str = "zk-configmap";

const TEMPLATE_FILE_EXTENSION: &str = ".yaml";

impl Template {
    pub fn new(path: &Path, config: Value) -> Result<Self> {
        let mut handlebars = Handlebars::new();
        handlebars.register_templates_directory(TEMPLATE_FILE_EXTENSION, path)?;
        handlebars.register_helper("get_files", Box::new(get_files_helper));
        handlebars.set_strict_mode(true);
        Ok(Template { handlebars, config })
    }

    pub fn nifi_statefulset(
        &self,
        name: &str,
        replicas: &u8,
        image_name: &Option<String>,
        storage_class: &Option<String>,
    ) -> Result<Option<String>> {
        let image = json!({ "image": image_name });
        self.statefulset(name, replicas, image, storage_class, NIFI_STATEFULSET)
    }

    pub fn zk_statefulset(
        &self,
        name: &str,
        replicas: &u8,
        image_name: &Option<String>,
        storage_class: &Option<String>,
    ) -> Result<Option<String>> {
        let image = json!({ "zkImage": image_name });
        self.statefulset(name, replicas, image, storage_class, ZK_STATEFULSET)
    }

    pub fn nifi_service(&self, name: &str) -> Result<Option<String>> {
        self.service(name, NIFI_SERVICE)
    }

    pub fn nifi_headless_service(&self, name: &str) -> Result<Option<String>> {
        self.service(name, NIFI_HEADLESS_SERVICE)
    }

    pub fn zk_service(&self, name: &str) -> Result<Option<String>> {
        self.service(name, ZK_SERVICE)
    }

    pub fn zk_headless_service(&self, name: &str) -> Result<Option<String>> {
        self.service(name, ZK_HEADLESS_SERVICE)
    }

    fn service(&self, name: &str, template: &str) -> Result<Option<String>> {
        let data = self.get_config(name);
        debug!("service template {} params\n: {}", &template, &data);
        self.render(&data, template)
    }

    pub fn ingress(&self, name: &str) -> Result<Option<String>> {
        let data = self.get_config(name);
        debug!("ingress template params\n: {}", &data);
        self.render(&data, INGRESS)
    }

    fn get_config(&self, name: &str) -> Value {
        let mut current_cfg = self.config.clone();
        let data = json!({ "name": name });
        Template::merge_json(&mut current_cfg, data);
        current_cfg
    }

    pub fn nifi_configmap(&self, name: &str) -> Result<Option<String>> {
        self.configmap(name, NIFI_CONFIGMAP)
    }

    pub fn zk_configmap(&self, name: &str) -> Result<Option<String>> {
        self.configmap(name, ZK_CONFIGMAP)
    }

    fn configmap(&self, name: &str, template: &str) -> Result<Option<String>> {
        let data = self.get_config(name);
        debug!("{} template params\n: {}", template, &data);
        self.render(&data, template)
    }

    fn render(&self, data: &Value, template: &str) -> Result<Option<String>> {
        self.handlebars
            .render(template, &data)
            .map_err(Error::new)
            .map(|s| if s.is_empty() { None } else { Some(s) })
    }

    fn statefulset(
        &self,
        name: &str,
        replicas: &u8,
        image: Value,
        storage_class: &Option<String>,
        template: &str,
    ) -> Result<Option<String>> {
        let mut data = json!({
            "name": name,
            "replicas": &replicas.to_string(),
            "storage_class": storage_class
        });
        Template::merge_json(&mut data, image);
        let mut current_cfg = self.config.clone();
        Template::merge_json(&mut current_cfg, data);
        debug!("{} template params\n: {}", &template, &current_cfg);
        self.render(&current_cfg, template)
    }

    fn merge_json(a: &mut Value, b: Value) {
        if let Value::Object(a) = a {
            if let Value::Object(b) = b {
                for (k, v) in b {
                    if v.is_null() {
                        a.remove(&k);
                    } else {
                        Template::merge_json(a.entry(k).or_insert(Value::Null), v);
                    }
                }
                return;
            }
        }
        *a = b;
    }
}
