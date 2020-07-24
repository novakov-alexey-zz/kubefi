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
const ZK_STATEFULSET: &str = "zk-statefulset";
const SERVICE: &str = "service";
const INGRESS: &str = "ingress";
const CONFIGMAP: &str = "configmap";
const TEMPLATE_FILE_EXTENSION: &str = ".yaml";

impl Template {
    pub fn new(path: &Path, config: Value) -> Result<Self> {
        let mut handlebars = Handlebars::new();
        handlebars.register_templates_directory(TEMPLATE_FILE_EXTENSION, path)?;
        handlebars.register_helper("get_files", Box::new(get_files_helper));
        handlebars.set_strict_mode(true);
        Ok(Template { handlebars, config })
    }

    pub fn nifi_statefulset(&self, name: &String, replicas: &u8, image_name: &Option<String>,
                            storage_class: &Option<String>) -> Result<Option<String>> {
        self.statefulset(name, replicas, image_name, storage_class, NIFI_STATEFULSET)
    }

    pub fn zk_statefulset(&self, name: &String, replicas: &u8, image_name: &Option<String>,
                          storage_class: &Option<String>) -> Result<Option<String>> {
        self.statefulset(name, replicas, image_name, storage_class, ZK_STATEFULSET)
    }

    pub fn service(&self, name: &String) -> Result<Option<String>> {
        let mut data = json!({"name": name});
        Template::merge(&mut data, self.config.clone());
        debug!("service template params\n: {}", &data);
        self.render(&data, SERVICE)
    }

    pub fn ingress(&self, name: &String) -> Result<Option<String>> {
        let mut data = json!({"name": name});
        Template::merge(&mut data, self.config.clone());
        debug!("ingress template params\n: {}", &data);
        self.render(&data, INGRESS)
    }

    pub fn configmap(&self, name: &String) -> Result<Option<String>> {
        let mut data = json!({"name": name});
        Template::merge(&mut data, self.config.clone());
        debug!("configmap template params\n: {}", &data);
        self.render(&data, CONFIGMAP)
    }

    fn render(&self, data: &Value, template: &str) -> Result<Option<String>> {
        self.handlebars.render(template, &data)
            .map_err(|e| Error::new(e))
            .and_then(|s| if s.is_empty() {
                Ok(None)
            } else {
                Ok(Some(s))
            })
    }

    fn statefulset(&self, name: &String, replicas: &u8, image_name: &Option<String>,
                   storage_class: &Option<String>, template: &str) -> Result<Option<String>> {
        let mut data = json!({
            "name": name,
            "image" : image_name,
            "replicas": &replicas.to_string(),
            "storage_class": storage_class
        });
        Template::merge(&mut data, self.config.clone());
        debug!("statefulset template params\n: {}", &data);
        self.render(&data, template)
    }

    fn merge(a: &mut Value, b: Value) {
        if let Value::Object(a) = a {
            if let Value::Object(b) = b {
                for (k, v) in b {
                    if v.is_null() {
                        a.remove(&k);
                    } else {
                        Template::merge(a.entry(k).or_insert(Value::Null), v);
                    }
                }
                return;
            }
        }
        *a = b;
    }
}