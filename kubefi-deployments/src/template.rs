use std::collections::BTreeMap;
use std::path::Path;

use anyhow::{Error, Result};
use handlebars::Handlebars;

pub struct Template {
    handlebars: Handlebars<'static>,
}

const NI_FI_STATEFULSET: &str = "nifi-statefulset";
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

    pub fn nifi_statefulset_for(&self, name: &String, replicas: &u8, image_name: &String,
                                storage_class: &String) -> Result<String> {
        let replicas_str = &replicas.to_string();
        let data: BTreeMap<&str, &String> = [
            ("name", name),
            ("imageName", image_name),
            ("replicas", replicas_str),
            ("storageClass", storage_class)].iter().cloned().collect();
        self.handlebars.render(NI_FI_STATEFULSET, &data)
            .map_err(|e| Error::new(e))
    }
}