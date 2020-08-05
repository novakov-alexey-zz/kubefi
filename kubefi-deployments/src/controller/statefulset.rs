use std::rc::Rc;

use anyhow::{Error, Result};
use k8s_openapi::api::apps::v1::StatefulSet;
use k8s_openapi::api::core::v1::Pod;
use kube::api::{DeleteParams, ListParams, PostParams};
use kube::Client;

use crate::controller::{delete_resources, from_yaml, get_api, get_or_create, KUBEFI_LABELS};
use crate::crd::NiFiDeployment;
use crate::template::Template;

use super::either::Either::Left;

pub struct StatefulSetController {
    pub client: Rc<Client>,
    pub template: Rc<Template>,
}

#[derive(Debug)]
struct SetParams {
    pub replicas: i32,
    pub container: String,
    pub image: Option<String>,
    pub set_name: String,
    pub app_label: String,
    pub storage_class: Option<String>,
}

impl StatefulSetController {
    async fn update_existing_set<F: FnOnce(&str, &NiFiDeployment) -> Result<Option<String>>>(
        &self,
        d: &NiFiDeployment,
        cr_name: &str,
        ns: &str,
        set: StatefulSet,
        set_params: &SetParams,
        get_yaml: F,
    ) -> Result<()> {
        let image_changed =
            self.image_changed(&set, &set_params.image.clone(), &set_params.container);
        let replicas_changed = self.scale_set(&set, set_params.replicas);
        let storage_class_changed = self.storage_class(&set, &set_params.storage_class);

        if storage_class_changed {
            let yaml = get_yaml(&cr_name, &d)?;
            self.recreate_set(&ns, &set_params, yaml).await?;
        } else {
            if image_changed || replicas_changed {
                debug!(
                    "Updating existing {} statefulset with: {:?}",
                    &set_params.set_name, &set_params
                );
                let yaml = get_yaml(&cr_name, &d)?;
                match yaml {
                    Some(t) => {
                        let new_set = from_yaml(&t)?;
                        let api = get_api::<StatefulSet>(&self.client, &ns);
                        let pp = PostParams::default();
                        api.replace(&set_params.set_name, &pp, &new_set)
                            .await
                            .map(|_| ())
                            .map_err(Error::from)
                    }
                    None => Ok(()),
                }?;
            }

            if image_changed {
                let params = &DeleteParams::default();
                let lp = ListParams::default()
                    .labels(format!("app={},{}", set_params.app_label, KUBEFI_LABELS).as_str());
                delete_resources::<Pod>(&self.client, &ns, &params, &lp).await?;
            }
        }

        Ok(())
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

    fn scale_set(&self, set: &StatefulSet, expected_replicas: i32) -> bool {
        let replicas = set.clone().spec.as_ref().and_then(|s| s.replicas);
        match replicas {
            Some(current_replicas) if current_replicas != expected_replicas => true,
            _ => false,
        }
    }

    fn storage_class(&self, set: &StatefulSet, storage_class: &Option<String>) -> bool {
        match storage_class {
            Some(sc) => set
                .clone()
                .spec
                .and_then(|s| {
                    s.volume_claim_templates.map(|vc| {
                        vc.iter()
                            .find(|pvc| {
                                pvc.spec
                                    .clone()
                                    .into_iter()
                                    .find(|spec| {
                                        spec.clone()
                                            .storage_class_name
                                            .map(|scn| &scn != sc)
                                            .unwrap_or(false)
                                    })
                                    .is_some()
                            })
                            .is_some()
                    })
                })
                .unwrap_or(false),
            None => false,
        }
    }

    pub fn nifi_template(&self, name: &str, d: &NiFiDeployment) -> Result<Option<String>> {
        self.template.nifi_statefulset(
            &name,
            &d.spec.nifi_replicas,
            &d.spec.image,
            &d.spec.storage_class,
        )
    }

    pub fn zk_template(&self, name: &str, d: &NiFiDeployment) -> Result<Option<String>> {
        self.template.zk_statefulset(
            &name,
            &d.spec.zk_replicas,
            &d.spec.zk_image,
            &d.spec.storage_class,
        )
    }

    fn zk_set_name(name: &str) -> String {
        format!("{}-zookeeper", &name)
    }

    pub async fn handle_sets(&self, d: &NiFiDeployment, name: &str, ns: &str) -> Result<()> {
        let nifi = get_or_create::<StatefulSet, _>(&self.client, &name, &name, &ns, |name| {
            self.nifi_template(&name, &d)
        });
        let zk_set_name = StatefulSetController::zk_set_name(&name);
        let zk = get_or_create::<StatefulSet, _>(&self.client, &zk_set_name, &name, &ns, |name| {
            self.zk_template(&name, &d)
        });
        let (r1, r2) = futures::future::join(nifi, zk).await;

        if let Left(Some(set)) = r1? {
            let params = SetParams {
                replicas: d.clone().spec.nifi_replicas as i32,
                container: "server".to_string(),
                image: d.clone().spec.image,
                set_name: name.to_string(),
                app_label: "nifi".to_string(),
                storage_class: d.clone().spec.storage_class,
            };
            self.update_existing_set(&d, &name, &ns, set, &params, |cr_name, deployment| {
                self.nifi_template(&cr_name, &deployment)
            })
            .await?
        }

        if let Left(Some(set)) = r2? {
            let params = SetParams {
                replicas: d.clone().spec.zk_replicas as i32,
                container: "zookeeper".to_string(),
                image: d.clone().spec.zk_image,
                set_name: zk_set_name,
                app_label: "zookeeper".to_string(),
                storage_class: d.clone().spec.storage_class,
            };
            self.update_existing_set(&d, &name, &ns, set, &params, |cr_name, deployment| {
                self.zk_template(&cr_name, &deployment)
            })
            .await?
        }
        Ok(())
    }

    fn image_changed(&self, set: &StatefulSet, image: &Option<String>, container: &str) -> bool {
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
}
