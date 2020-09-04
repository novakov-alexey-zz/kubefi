use std::rc::Rc;

use anyhow::Result;
use k8s_openapi::api::core::v1::Service;
use k8s_openapi::api::extensions::v1beta1::Ingress;
use kube::api::DeleteParams;
use kube::Client;

use crate::controller::{create_from_yaml, get_api, get_or_create};
use crate::crd::IngressCfg;
use crate::template::Template;

use super::either::Either;
use super::either::Either::{Left, Right};

pub struct ServiceController {
    pub client: Rc<Client>,
    pub template: Rc<Template>,
}

impl ServiceController {
    pub async fn handle_services(
        &self,
        name: &str,
        ns: &str,
        ingress_cfg: &Option<IngressCfg>,
    ) -> Result<bool> {
        let svc = get_or_create::<Service, _>(&self.client, &name, &name, &ns, |name| {
            self.template.nifi_service(name)
        });

        let headless_svc_name = format!("{}-headless", &name);
        let headless_svc =
            get_or_create::<Service, _>(&self.client, &headless_svc_name, &name, &ns, |name| {
                self.template.nifi_headless_service(name)
            });

        let zk_svc_name = format!("{}-zookeeper", &name);
        let zk_svc = get_or_create::<Service, _>(&self.client, &zk_svc_name, &name, &ns, |name| {
            self.template.zk_service(name)
        });

        let zk_headless_svc_name = format!("{}-zookeeper-headless", &name);
        let zk_headless_svc =
            get_or_create::<Service, _>(&self.client, &zk_headless_svc_name, &name, &ns, |name| {
                self.template.zk_headless_service(name)
            });

        let ingress_name = format!("{}-ingress", &name);
        let ingress =
            get_or_create::<Ingress, _>(&self.client, &ingress_name, &name, &ns, |name| {
                self.template.ingress(name, &ingress_cfg)
            });

        let (svc, headless_svc, zk_svc, zk_headless_svc, ingress) =
            futures::future::join5(svc, headless_svc, zk_svc, zk_headless_svc, ingress).await;

        let ingress_updated = self
            .handle_update(&name, &ns, &ingress_cfg, &ingress_name, ingress)
            .await;
        vec![svc, headless_svc, zk_svc, zk_headless_svc]
            .into_iter()
            .fold(Ok(false), |acc, res| {
                let resource = res?;
                acc.map(|a| a || resource_updated(resource))
            })
            .and_then(|svc_updated| ingress_updated.map(|upd| upd || svc_updated))
    }

    async fn handle_update(
        &self,
        name: &str,
        ns: &str,
        ingress_cfg: &Option<IngressCfg>,
        ingress_name: &str,
        ingress: Result<Either<Option<Ingress>, Option<Ingress>>>,
    ) -> Result<bool> {
        let ingress_changed = ingress_updated(ingress, &ingress_cfg);
        match ingress_changed {
            Ok(true) => self
                .recreate_ingress(&name, &ns, &ingress_name, &ingress_cfg)
                .await
                .map(|_| true),
            Ok(_) => Ok(false),
            Err(e) => Err(e),
        }
    }

    async fn recreate_ingress(
        &self,
        cr_name: &str,
        ns: &str,
        ingress_name: &str,
        ingress_cfg: &Option<IngressCfg>,
    ) -> Result<()> {
        let params = &DeleteParams::default();
        let api = get_api::<Ingress>(&self.client, &ns);
        api.delete(&ingress_name, params).await?;

        debug!("Creating new Ingress: {}", &ingress_name);
        create_from_yaml::<Ingress, _, _>(
            &cr_name,
            &ns,
            &self.client,
            |name| self.template.ingress(name, ingress_cfg),
            Ok,
        )
        .await
        .map(|_| ())
    }
}

fn ingress_updated(
    current_ingress: Result<Either<Option<Ingress>, Option<Ingress>>>,
    ingress_cfg: &Option<IngressCfg>,
) -> Result<bool> {
    match ingress_cfg {
        Some(cfg) => {
            debug!("Ingress config: {:?}", &cfg);
            current_ingress.map(|r| match r {
                Left(Some(ing)) => {
                    debug!("ing spec: {:?}", &ing.spec);
                    let found = ing
                        .spec
                        .and_then(|s| s.rules)
                        .unwrap_or_default()
                        .iter()
                        .any(|r| {
                            r.host
                                .as_ref()
                                .map(|h| h == cfg.host.as_str())
                                .unwrap_or(false)
                        });
                    !found
                }
                _ => false,
            })
        }
        None => Ok(false),
    }
}

fn resource_updated<T>(result: Either<Option<T>, Option<T>>) -> bool {
    match result {
        Left(Some(_)) => false,
        Right(Some(_)) => true,
        _ => false,
    }
}
