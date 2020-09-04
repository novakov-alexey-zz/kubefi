use std::rc::Rc;

use anyhow::{Error, Result};
use k8s_openapi::api::apps::v1::StatefulSet;
use k8s_openapi::api::core::v1::Pod;
use kube::api::{DeleteParams, ListParams, PostParams};
use kube::Client;

use crate::controller::{
    delete_resources, from_yaml, get_api, get_or_create, ConfigMapState, KUBEFI_LABELS,
    NIFI_APP_LABEL, ZK_APP_LABEL,
};
use crate::crd::NiFiDeployment;
use crate::template::Template;

use super::either::Either::{Left, Right};

pub struct StatefulSetController {
    pub client: Rc<Client>,
    pub template: Rc<Template>,
}

#[derive(Debug, Clone)]
struct SetParams {
    pub replicas: i32,
    pub container: String,
    pub image: Option<String>,
    pub set_name: String,
    pub app_label: String,
    pub storage_class: Option<String>,
    pub cm_state: Option<ConfigMapState>,
    pub svc_updated: bool,
}

const LOGGING_VOLUME: &str = "logback-xml";
const NIFI_CONTAINER_NAME: &str = "server";
const ZOOKEEPER_CONTAINER_NAME: &str = "zookeeper";

impl StatefulSetController {
    async fn update_existing_set<F: FnOnce(&str, &NiFiDeployment) -> Result<Option<String>>>(
        &self,
        d: &NiFiDeployment,
        cr_name: &str,
        ns: &str,
        set: StatefulSet,
        params: &SetParams,
        get_yaml: F,
    ) -> Result<bool> {
        let image_changed = image_changed(&set, &params.image.clone(), &params.container);
        let replicas_changed = scale_set(&set, params.replicas);
        let storage_class_changed = storage_class(&set, &params.storage_class);
        let logging_cm_changed =
            logging_cm(&set, params.clone().cm_state.and_then(|cm| cm.logging_cm));

        if storage_class_changed {
            let yaml = get_yaml(&cr_name, &d)?;
            self.recreate_set(&ns, &params, yaml).await?;
        } else {
            if image_changed || replicas_changed || logging_cm_changed {
                let reason = format!(
                    "image_changed: {}, replicas_changed: {}, logging_cm_changed: {}",
                    image_changed, replicas_changed, logging_cm_changed
                );
                debug!(
                    "Updating existing {} statefulset with: {:?}. Reason: {}",
                    &params.set_name, &params, reason
                );
                let yaml = get_yaml(&cr_name, &d)?;
                match yaml {
                    Some(y) => self.replace_set(&ns, &params, &y).await,
                    None => Ok(()),
                }?;
            }

            if image_changed
                || params
                    .cm_state
                    .clone()
                    .map(|cm| cm.updated)
                    .unwrap_or(false)
            {
                self.remove_pods(&ns, params, image_changed).await?;
            }
        }
        let state_changed =
            storage_class_changed || image_changed || replicas_changed || logging_cm_changed;
        Ok(state_changed)
    }

    async fn remove_pods(&self, ns: &str, params: &SetParams, image_changed: bool) -> Result<()> {
        let dp = &DeleteParams::default();
        let labels = format!("app={},{}", params.app_label, KUBEFI_LABELS);
        let lp = ListParams::default().labels(&labels);
        debug!(
            "Removing all Pod(s) with: {:?}. Reason: image changed = {}, configMap changed = {}",
            labels,
            image_changed,
            params
                .cm_state
                .clone()
                .map(|cm| cm.updated)
                .unwrap_or(false)
        );
        delete_resources::<Pod>(&self.client, &ns, &dp, &lp).await
    }

    async fn replace_set(&self, ns: &str, set_params: &SetParams, yaml: &str) -> Result<(), Error> {
        let new_set = from_yaml(&yaml)?;
        let api = get_api::<StatefulSet>(&self.client, &ns);
        let pp = PostParams::default();
        api.replace(&set_params.set_name, &pp, &new_set)
            .await
            .map(|_| ())
            .map_err(Error::from)
    }

    async fn recreate_set(
        &self,
        ns: &str,
        set_params: &SetParams,
        yaml: Option<String>,
    ) -> Result<()> {
        match yaml {
            Some(t) => {
                let new_set = from_yaml(&t)?;
                let api = get_api::<StatefulSet>(&self.client, &ns);
                let dp = DeleteParams::default();
                api.delete(&set_params.set_name, &dp)
                    .await
                    .map(|_| ())
                    .map_err(Error::from)?;
                let pp = PostParams::default();
                api.create(&pp, &new_set).await.map(|_| ())
            }
            None => Ok(()),
        }?;
        Ok(())
    }

    pub fn nifi_template(&self, name: &str, d: &NiFiDeployment) -> Result<Option<String>> {
        self.template.nifi_statefulset(&name, &d.spec)
    }

    pub fn zk_template(&self, name: &str, d: &NiFiDeployment) -> Result<Option<String>> {
        self.template.zk_statefulset(
            &name,
            &d.spec.zk.replicas,
            &d.spec.zk.image,
            &d.spec.storage_class,
        )
    }

    pub async fn handle_sets(
        &self,
        d: &NiFiDeployment,
        name: &str,
        ns: &str,
        nifi_cm_state: ConfigMapState,
        service_updated: bool,
    ) -> Result<bool> {
        let nifi = get_or_create::<StatefulSet, _>(&self.client, &name, &name, &ns, |name| {
            self.nifi_template(&name, &d)
        });
        let zk_set_name = zk_set_name(&name);
        let get_yaml = |name: &str| self.zk_template(&name, &d);
        let zk = get_or_create::<StatefulSet, _>(&self.client, &zk_set_name, &name, &ns, get_yaml);
        let (nifi_res, zk_res) = futures::future::join(nifi, zk).await;

        let nifi_updated = match nifi_res? {
            Left(Some(existing_set)) => {
                let params = SetParams {
                    replicas: d.clone().spec.nifi_replicas as i32,
                    container: NIFI_CONTAINER_NAME.to_string(),
                    image: d.clone().spec.image,
                    set_name: name.to_string(),
                    app_label: NIFI_APP_LABEL.to_string(),
                    storage_class: d.clone().spec.storage_class,
                    cm_state: Some(nifi_cm_state.clone()),
                    svc_updated: service_updated,
                };
                self.update_existing_set(
                    &d,
                    &name,
                    &ns,
                    existing_set,
                    &params,
                    |cr_name, deployment| self.nifi_template(&cr_name, &deployment),
                )
                .await
            }
            Right(Some(_)) => Ok(true),
            _ => Ok(false),
        };

        let zk_updated = match zk_res? {
            Left(Some(existing_set)) if nifi_updated.is_ok() => {
                let params = SetParams {
                    replicas: d.clone().spec.zk.replicas as i32,
                    container: ZOOKEEPER_CONTAINER_NAME.to_string(),
                    image: d.clone().spec.zk.image,
                    set_name: zk_set_name,
                    app_label: ZK_APP_LABEL.to_string(),
                    storage_class: d.clone().spec.storage_class,
                    cm_state: None,
                    svc_updated: false,
                };
                self.update_existing_set(
                    &d,
                    &name,
                    &ns,
                    existing_set,
                    &params,
                    |cr_name, deployment| self.zk_template(&cr_name, &deployment),
                )
                .await
            }
            Right(Some(_)) => Ok(true),
            _ => Ok(false),
        };

        nifi_updated.and(zk_updated)
    }
}

fn zk_set_name(name: &str) -> String {
    format!("{}-zookeeper", &name)
}

fn image_changed(set: &StatefulSet, image: &Option<String>, container: &str) -> bool {
    match image {
        Some(target_image) => set
            .clone()
            .spec
            .and_then(|s| {
                s.template.spec.and_then(|spec| {
                    spec.containers.into_iter().find(|c| {
                        c.name == container && c.image.iter().any(|img| img != target_image)
                    })
                })
            })
            .is_some(),
        None => false,
    }
}

fn scale_set(set: &StatefulSet, expected_replicas: i32) -> bool {
    let replicas = set.clone().spec.as_ref().and_then(|s| s.replicas);
    match replicas {
        Some(current_replicas) if current_replicas != expected_replicas => true,
        _ => false,
    }
}

fn storage_class(set: &StatefulSet, storage_class: &Option<String>) -> bool {
    match storage_class {
        Some(sc) => set
            .clone()
            .spec
            .and_then(|s| {
                s.volume_claim_templates.map(|vc| {
                    vc.iter().any(|pvc| {
                        pvc.spec.clone().into_iter().any(|spec| {
                            spec.storage_class_name
                                .map(|scn| &scn != sc)
                                .unwrap_or(false)
                        })
                    })
                })
            })
            .unwrap_or(false),
        None => false,
    }
}

fn logging_cm(set: &StatefulSet, logging_cm: Option<String>) -> bool {
    match logging_cm {
        Some(logging_cm_name) => {
            let found = set
                .clone()
                .spec
                .and_then(|s| {
                    s.template.spec.and_then(|ss| {
                        let volumes = ss.volumes.unwrap_or_else(Vec::new);
                        let found =
                            volumes
                                .iter()
                                .find(|v| v.name == LOGGING_VOLUME)
                                .and_then(|v| {
                                    let current_name =
                                        v.config_map.as_ref().and_then(|cm| cm.name.clone());
                                    current_name.filter(|n| n == &logging_cm_name)
                                });
                        found
                    })
                })
                .is_some();
            !found
        }
        None => false,
    }
}
