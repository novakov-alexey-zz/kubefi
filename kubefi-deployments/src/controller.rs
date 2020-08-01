extern crate anyhow;
extern crate either;
extern crate kube;
extern crate kube_derive;
extern crate serde;

use std::fmt::Debug;
use std::path::Path;
use std::{error, fmt};

use anyhow::Error;
use k8s_openapi::api::apps::v1::StatefulSet;
use k8s_openapi::api::core::v1::{ConfigMap, Pod, Service};
use k8s_openapi::api::extensions::v1beta1::Ingress;
use k8s_openapi::Resource;
use kube::api::{DeleteParams, ListParams, Meta, PostParams};
use kube::{Api, Client};
use serde::de::DeserializeOwned;
use serde::Serialize;
use serde_json::Value;

use crate::anyhow::Result;
use crate::controller::ControllerError::MissingProperty;
use crate::crd::{NiFiDeployment, NiFiDeploymentStatus};
use crate::template::Template;
use crate::Namespace;

use self::either::Either;
use self::either::Either::{Left, Right};

const KUBEFI_LABELS: &str = "app.kubernetes.io/managed-by=Kubefi,release=nifi";

#[derive(Debug)]
pub enum ControllerError {
    MissingProperty(String, String),
}

#[derive(Serialize, Debug, Clone)]
pub struct ReplaceStatus {
    pub name: String,
    pub ns: String,
    pub status: NiFiDeploymentStatus,
}

impl fmt::Display for ControllerError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            ControllerError::MissingProperty(property, kind) => write!(
                f,
                "Property {:?} for {} resource is missing",
                property, kind
            ),
        }
    }
}

impl error::Error for ControllerError {
    fn source(&self) -> Option<&(dyn error::Error + 'static)> {
        match *self {
            ControllerError::MissingProperty(_, _) => None,
        }
    }
}

pub struct NiFiController {
    pub namespace: Namespace,
    pub client: Client,
    template: Template,
}

#[derive(Debug)]
pub struct SetParams {
    pub replicas: i32,
    pub container: String,
    pub image: Option<String>,
    pub set_name: String,
    pub delete_pods: bool,
    pub app_label: String,
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

    pub async fn on_apply(&self, d: NiFiDeployment) -> Result<Option<ReplaceStatus>> {
        let name = d
            .clone()
            .metadata
            .name
            .ok_or_else(|| MissingProperty("name".to_string(), d.kind.clone()))?;
        let ns = NiFiController::read_namespace(&d)?;
        let error = self
            .handle_event(d.clone(), &name, &ns)
            .await
            .err()
            .map(|e| e.to_string())
            .unwrap_or_default();
        let status = NiFiDeploymentStatus {
            nifi_replicas: d.spec.nifi_replicas,
            error_msg: error,
        };
        Ok(Some(ReplaceStatus { name, ns, status }))
    }

    pub async fn on_delete(&self, d: NiFiDeployment) -> Result<()> {
        let ns = NiFiController::read_namespace(&d)?;
        let params = &DeleteParams::default();
        let lp = ListParams::default().labels(KUBEFI_LABELS);

        let sts = self.delete_resources::<StatefulSet>(&ns, &params, &lp);
        let svc = self.delete_resources::<Service>(&ns, &params, &lp);
        let cm = self.delete_resources::<ConfigMap>(&ns, &params, &lp);
        let ing = self.delete_resources::<Ingress>(&ns, &params, &lp);
        let (r1, r2, r3, r4) = futures::future::join4(sts, svc, cm, ing).await;
        r1.and(r2).and(r3).and(r4)
    }

    async fn delete_resources<T: Resource + Clone + DeserializeOwned + Meta + Debug>(
        &self,
        ns: &str,
        params: &DeleteParams,
        lp: &ListParams,
    ) -> Result<()> {
        let names = self.find_names::<T>(&ns, &lp).await?;
        debug!("Resources to delete: {:?}", &names);
        let api = self.get_api::<T>(&ns);
        let deletes = names.iter().map(|name| api.delete(&name, &params));
        futures::future::join_all(deletes)
            .await
            .into_iter()
            .map(|r| {
                r.map(|e| {
                    e.map_left(|resource| debug!("Deleted {}", Meta::name(&resource)))
                        .map_right(|status| debug!("Deleting {:?}", status))
                })
                .map(|_| ())
            })
            .fold(Ok(()), |acc, r| acc.and(r.map_err(Error::from)))
    }

    async fn find_names<T: Resource + Clone + DeserializeOwned + Meta>(
        &self,
        ns: &str,
        lp: &ListParams,
    ) -> Result<Vec<String>> {
        let api: Api<T> = self.get_api(&ns);
        let list = &api.list(&lp).await?;
        let names = list.into_iter().map(Meta::name).collect();
        Ok(names)
    }

    fn get_api<T: Resource>(&self, ns: &str) -> Api<T> {
        Api::namespaced(self.client.clone(), &ns)
    }

    fn nifi_template(&self, name: &str, d: &NiFiDeployment) -> Result<Option<String>> {
        self.template.nifi_statefulset(
            &name,
            &d.spec.nifi_replicas,
            &d.spec.image,
            &d.spec.storage_class,
        )
    }

    fn zk_template(&self, name: &str, d: &NiFiDeployment) -> Result<Option<String>> {
        self.template.zk_statefulset(
            &name,
            &d.spec.zk_replicas,
            &d.spec.zk_image,
            &d.spec.storage_class,
        )
    }

    async fn handle_event(&self, d: NiFiDeployment, name: &str, ns: &str) -> Result<()> {
        self.handle_configmaps(&name, &ns).await?;
        self.handle_sets(&d, &name, &ns).await?;
        self.handle_services(&name, &ns).await
    }

    async fn handle_services(&self, name: &&str, ns: &&str) -> Result<(), Error> {
        let svc = self.get_or_create::<Service, _>(&name, &name, &ns, |name| {
            self.template.nifi_service(name)
        });

        let headless_svc_name = format!("{}-headless", &name);
        let headless_svc =
            self.get_or_create::<Service, _>(&headless_svc_name, &name, &ns, |name| {
                self.template.nifi_headless_service(name)
            });

        let zk_svc_name = format!("{}-zookeeper", &name);
        let zk_svc = self.get_or_create::<Service, _>(&zk_svc_name, &name, &ns, |name| {
            self.template.zk_service(name)
        });

        let zk_headless_svc_name = format!("{}-zookeeper-headless", &name);
        let zk_headless_svc =
            self.get_or_create::<Service, _>(&zk_headless_svc_name, &name, &ns, |name| {
                self.template.zk_headless_service(name)
            });

        let ingress_name = format!("{}-ingress", &name);
        let ingress = self.get_or_create::<Ingress, _>(&ingress_name, &name, &ns, |name| {
            self.template.ingress(name)
        });

        let (r1, r2, r3, r4, r5) =
            futures::future::join5(svc, headless_svc, zk_svc, zk_headless_svc, ingress).await;
        r1.and(r2).and(r3).and(r4).and(r5).map(|_| ())
    }

    async fn handle_sets(&self, d: &NiFiDeployment, name: &str, ns: &str) -> Result<()> {
        let nifi = self.get_or_create::<StatefulSet, _>(&name, &name, &ns, |name| {
            self.nifi_template(&name, &d)
        });
        let zk_set_name = NiFiController::zk_set_name(&name);
        let zk = self.get_or_create::<StatefulSet, _>(&zk_set_name, &name, &ns, |name| {
            self.zk_template(&name, &d)
        });
        let (r1, r2) = futures::future::join(nifi, zk).await;

        if let Left(Some(set)) = r1? {
            let params = SetParams {
                replicas: d.clone().spec.nifi_replicas as i32,
                container: "server".to_string(),
                image: d.clone().spec.image,
                set_name: name.to_string(),
                delete_pods: false,
                app_label: "nifi".to_string(),
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
                delete_pods: true,
                app_label: "zookeeper".to_string(),
            };
            self.update_existing_set(&d, &name, &ns, set, &params, |cr_name, deployment| {
                self.zk_template(&cr_name, &deployment)
            })
            .await?
        }
        Ok(())
    }

    async fn update_existing_set<F: FnOnce(&str, &NiFiDeployment) -> Result<Option<String>>>(
        &self,
        d: &NiFiDeployment,
        cr_name: &str,
        ns: &str,
        set: StatefulSet,
        set_params: &SetParams,
        get_yaml: F,
    ) -> Result<()> {
        let (image_changed, replicas_changed) = self.updated(set, &set_params);

        if image_changed || replicas_changed {
            debug!(
                "Updating existing {} statefulset with: {:?}",
                &set_params.set_name, &set_params
            );
            let yaml = get_yaml(&cr_name, &d)?;
            match yaml {
                Some(t) => {
                    let new_set = NiFiController::from_yaml(&t)?;
                    let api = self.get_api::<StatefulSet>(&ns);
                    let pp = PostParams::default();
                    api.replace(&set_params.set_name, &pp, &new_set)
                        .await
                        .map(|_| ())
                        .map_err(Error::from)
                }
                None => Ok(()),
            }?;
        }
        if image_changed && set_params.delete_pods {
            let params = &DeleteParams::default();
            let lp = ListParams::default().labels(
                format!("app={},heritage=Kubefi,release=nifi", set_params.app_label).as_str(),
            );
            self.delete_resources::<Pod>(&ns, &params, &lp).await?;
        }
        Ok(())
    }

    async fn handle_configmaps(
        &self,
        name: &str,
        ns: &str,
    ) -> Result<Either<Option<ConfigMap>, Option<ConfigMap>>> {
        let zk_cm_name = format!("{}-zookeeper", &name);
        let zk_cm = self.get_or_create::<ConfigMap, _>(&zk_cm_name, &name, &ns, |name| {
            self.template.zk_configmap(name)
        });

        let nifi_cm_name = format!("{}-config", &name);
        let nifi_cm = self.get_or_create::<ConfigMap, _>(&nifi_cm_name, &name, &ns, |name| {
            self.template.nifi_configmap(name)
        });

        let (r1, r2) = futures::future::join(zk_cm, nifi_cm).await;
        r1.and(r2)
    }

    fn zk_set_name(name: &str) -> String {
        format!("{}-zookeeper", &name)
    }

    fn read_namespace(d: &NiFiDeployment) -> Result<String, Error> {
        d.clone()
            .metadata
            .namespace
            .ok_or_else(|| Error::from(MissingProperty("namespace".to_string(), d.kind.clone())))
    }

    fn updated(&self, set: StatefulSet, params: &SetParams) -> (bool, bool) {
        (
            self.image_changed(&set, &params.image.clone(), &params.container),
            self.scale_set(set, params.replicas),
        )
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

    fn scale_set(&self, set: StatefulSet, expected_replicas: i32) -> bool {
        let replicas = set.spec.and_then(|s| s.replicas);
        match replicas {
            Some(current_replicas) if current_replicas != expected_replicas => true,
            _ => false,
        }
    }

    async fn get_or_create<
        T: Resource + Serialize + Clone + DeserializeOwned + Meta,
        F: FnOnce(&str) -> Result<Option<String>>,
    >(
        &self,
        name: &str,
        cr_name: &str,
        ns: &str,
        get_yaml: F,
    ) -> Result<Either<Option<T>, Option<T>>> {
        let api = self.get_api::<T>(&ns);
        match api.get(&name).await {
            Err(_) => {
                let yaml = get_yaml(&cr_name)?;
                match yaml {
                    Some(y) => {
                        let resource = NiFiController::from_yaml(&y)?;
                        self.create_resource(&api, resource)
                            .await
                            .map(Some)
                            .map(Right)
                    }
                    None => {
                        debug!("Resource template {} is not enabled or missing ", &name);
                        Ok(Right(None))
                    }
                }
            }
            Ok(res) => {
                debug!("Found existing resource {:?}", &name);
                Ok(Left(Some(res)))
            }
        }
    }

    fn from_yaml<T: Resource + Serialize + Clone + DeserializeOwned + Meta>(
        y: &str,
    ) -> Result<T, Error> {
        serde_yaml::from_str(&y).map_err(Error::new)
    }

    async fn create_resource<T: Serialize + Clone + DeserializeOwned + Meta>(
        &self,
        api: &Api<T>,
        resource: T,
    ) -> Result<T> {
        let pp = PostParams::default();
        api.create(&pp, &resource).await.map_err(Error::new)
    }
}
