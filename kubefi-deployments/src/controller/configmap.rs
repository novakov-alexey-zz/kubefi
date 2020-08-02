use anyhow::Result;
use k8s_openapi::api::core::v1::ConfigMap;
use kube::Client;

use crate::controller::get_or_create;
use crate::template::Template;

use super::either::Either;
use std::rc::Rc;

pub struct ConfigMapController {
    pub client: Rc<Client>,
    pub template: Rc<Template>,
}

impl<'a> ConfigMapController {
    pub async fn handle_configmaps(
        &self,
        name: &str,
        ns: &str,
    ) -> Result<Either<Option<ConfigMap>, Option<ConfigMap>>> {
        let zk_cm_name = format!("{}-zookeeper", &name);
        let zk_cm = get_or_create::<ConfigMap, _>(&self.client, &zk_cm_name, &name, &ns, |name| {
            self.template.zk_configmap(name)
        });

        let nifi_cm_name = format!("{}-config", &name);
        let nifi_cm =
            get_or_create::<ConfigMap, _>(&self.client, &nifi_cm_name, &name, &ns, |name| {
                self.template.nifi_configmap(name)
            });

        let (r1, r2) = futures::future::join(zk_cm, nifi_cm).await;
        r1.and(r2)
    }
}
