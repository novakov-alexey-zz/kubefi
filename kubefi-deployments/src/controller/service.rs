use anyhow::Error;
use k8s_openapi::api::core::v1::Service;
use k8s_openapi::api::extensions::v1beta1::Ingress;
use kube::Client;

use crate::controller::get_or_create;
use crate::template::Template;
use std::rc::Rc;

pub struct ServiceController {
    pub client: Rc<Client>,
    pub template: Rc<Template>,
}

impl ServiceController {
    pub async fn handle_services(&self, name: &str, ns: &str) -> Result<(), Error> {
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
                self.template.ingress(name)
            });

        let (r1, r2, r3, r4, r5) =
            futures::future::join5(svc, headless_svc, zk_svc, zk_headless_svc, ingress).await;
        r1.and(r2).and(r3).and(r4).and(r5).map(|_| ())
    }
}
