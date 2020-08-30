use crate::crd::NiFiDeploymentSpec;
use crate::crd::PodResources;
use std::path::Path;

use anyhow::{Error, Result};
use handlebars::Handlebars;
use serde_json::Value;

use crate::crd::AuthLdap;
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
    pub fn new(path: &Path, config: Value) -> Result<Template> {
        let mut handlebars = Handlebars::new();
        handlebars.register_templates_directory(TEMPLATE_FILE_EXTENSION, path)?;
        handlebars.register_helper("get_files", Box::new(get_files_helper));
        handlebars.set_strict_mode(true);
        Ok(Template { handlebars, config })
    }

    pub fn nifi_statefulset(
        &self,
        name: &str,
        spec: &NiFiDeploymentSpec,
    ) -> Result<Option<String>> {
        let mut data = json!({ "image": spec.image });
        let logging_cm_name = &spec
            .logging_config_map
            .clone()
            .unwrap_or(format!("{}-config", &name));
        let logging_data = json!({ "logging-configmap": logging_cm_name });
        Template::merge_json(&mut data, logging_data);

        if let Some(res) = &spec.nifi_resources {
            if let Some(jvm_heap_size) = &res.jvm_heap_size {
                Template::merge_json(
                    &mut data,
                    json!({ "nifiResources": {
                     "jvmHeapSize": jvm_heap_size
                    }}),
                );
            }
            
            let requests = self.get_pod_resources(&res.requests, "requests");
            Template::merge_json(&mut data, requests);

            let limits = self.get_pod_resources(&res.limits, "limits");
            Template::merge_json(&mut data, limits);
        }

        self.statefulset(
            name,
            &spec.nifi_replicas,
            data,
            &spec.storage_class,
            NIFI_STATEFULSET,
        )
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
        debug!("service template {} params\n:{}", &template, &data);
        self.render(&data, template)
    }

    pub fn ingress(&self, name: &str) -> Result<Option<String>> {
        let data = self.get_config(name);
        debug!("ingress template params\n:{}", &data);
        self.render(&data, INGRESS)
    }

    fn get_config(&self, name: &str) -> Value {
        let mut current_cfg = self.config.clone();
        let data = json!({ "name": name });
        Template::merge_json(&mut current_cfg, data);
        current_cfg
    }

    pub fn nifi_configmap(
        &self,
        name: &str,
        ns: &str,
        replicas: &u8,
        ldap: &Option<AuthLdap>,
    ) -> Result<Option<String>> {
        let mut data = self.get_config(name);

        let replica_indices = if replicas > &0 {
            (0..*replicas).collect::<Vec<_>>()
        } else {
            vec![]
        };
        Template::merge_json(
            &mut data,
            json!({ "ns": ns, "nifiReplicas": replica_indices}),
        );

        let maybe_ldap = &ldap.as_ref().map(|al| {
            json!(
            {
            "auth": {
            "ldap": {
                "host": al.host,
                "enabled": true
            }}
            }
            )
        });
        if let Some(cfg) = maybe_ldap {
            Template::merge_json(&mut data, cfg.clone());
        }

        self.configmap(NIFI_CONFIGMAP, &data)
    }

    fn get_pod_resources(
        &self,        
        pod_res: &Option<PodResources>,
        resource_name: &str,
    ) -> Value {
        let mut data = json!({});
        if let Some(res) = &pod_res {
            if let Some(cpu) = &res.cpu {
                Template::merge_json(
                    &mut data,
                    json!({ "nifiResources": {
                     resource_name: {
                         "cpu": cpu
                     }
                    }}),
                );
            };
            if let Some(memory) = &res.memory {
                Template::merge_json(
                    &mut data,
                    json!({ "nifiResources": {
                    resource_name: {
                         "memory": memory
                     }
                    }}),
                );
            };
        }
        data
    }

    pub fn zk_configmap(&self, name: &str) -> Result<Option<String>> {
        let data = self.get_config(name);
        self.configmap(ZK_CONFIGMAP, &data)
    }

    fn configmap(&self, template: &str, data: &Value) -> Result<Option<String>> {
        println!("{} template params\n:{}", template, &data);
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
        set_properties: Value,
        storage_class: &Option<String>,
        template: &str,
    ) -> Result<Option<String>> {
        let mut data = json!({
            "name": name,
            "replicas": &replicas.to_string()
        });
        Template::merge_json(&mut data, set_properties);

        if let Some(sc) = storage_class {
            let sc_json = json!({ "storageClass": sc });
            Template::merge_json(&mut data, sc_json);
        }

        let mut current_cfg = self.config.clone();
        Template::merge_json(&mut current_cfg, data);
        debug!("{} template params:\n{}", &template, &current_cfg);
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
