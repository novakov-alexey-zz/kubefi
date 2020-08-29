extern crate schemars;
extern crate serde_json;

use std::fmt::Debug;

use anyhow::Result;
use k8s_openapi::apiextensions_apiserver::pkg::apis::apiextensions::v1beta1::{
    CustomResourceDefinition, CustomResourceDefinitionSpec, CustomResourceValidation,
    JSONSchemaProps,
};
use kube::api::{DeleteParams, Meta, PostParams};
use kube::Api;
use kube_derive::CustomResource;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

pub const CRD_NAME: &str = "nifideployments.io.github.novakov-alexey";

#[derive(CustomResource, Serialize, Deserialize, Default, Clone, Debug, JsonSchema)]
#[kube(
    group = "io.github.novakov-alexey",
    version = "v1",
    namespaced,
    shortname = "nidp",
    status = "NiFiDeploymentStatus",
    printcolumn = r#"{"name":"Replicas", "jsonPath": ".spec.nifiReplicas", "type": "integer"}"#,
    apiextensions = "v1beta1"
)]
#[kube(
    scale = r#"{"specReplicasPath":".spec.nifiReplicas", "statusReplicasPath":".status.nifiReplicas"}"#
)]
#[serde(rename_all = "camelCase")]
pub struct NiFiDeploymentSpec {
    pub nifi_replicas: u8,
    pub zk_replicas: u8,
    pub image: Option<String>,
    pub zk_image: Option<String>,
    pub storage_class: Option<String>,
    pub ldap: Option<AuthLdap>,
    pub logging_config_map: Option<String>,
    pub nifi_resources: Option<Resources>,
}

#[derive(Deserialize, Serialize, Clone, Debug, Default, JsonSchema)]
pub struct AuthLdap {
    pub host: String,
}

#[derive(Deserialize, Serialize, Clone, Debug, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Resources {
    pub jvm_heap_size: Option<String>,
    pub requests: Option<PodResources>,
    pub limits: Option<PodResources>,
}

#[derive(Deserialize, Serialize, Clone, Debug, JsonSchema)]
pub struct PodResources {
    pub cpu: Option<String>,
    pub memory: Option<String>,
}

#[derive(Deserialize, Serialize, Clone, Debug, Default, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct NiFiDeploymentStatus {
    pub nifi_replicas: u8,
    pub error_msg: String,
}

pub async fn delete_old_version(crds: Api<CustomResourceDefinition>) -> Result<()> {
    let dp = DeleteParams::default();
    // but ignore delete err if not exists
    let deleted = crds.delete(CRD_NAME, &dp).await;
    deleted
        .map(|res| {
            res.map_left(|o| {
                info!(
                    "Deleting {}: ({:?})",
                    Meta::name(&o),
                    o.status.unwrap().conditions.unwrap().last()
                );
            })
            .map_right(|s| {
                // it's gone.
                info!("Deleted {:?}: ({:?})", CRD_NAME, s);
            });
        })
        .or(Ok(()))
}

pub async fn create_new_version(
    crds: Api<CustomResourceDefinition>,
    json_schema: String,
) -> Result<()> {
    let schema: JSONSchemaProps = serde_json::from_str(&json_schema)?;
    let crd = with_schema(schema, NiFiDeployment::crd());
    debug!("Creating CRD: {}", serde_json::to_string_pretty(&crd)?);
    let pp = PostParams::default();
    match crds.create(&pp, &crd).await {
        Ok(o) => {
            info!("Created {} ({:?})", Meta::name(&o), o.status.unwrap());
            Ok(())
        }
        Err(kube::Error::Api(ae)) => match ae.code {
            409 => Ok(()), // if delete is skipped
            _ => Err(ae.into()),
        },
        Err(e) => Err(e.into()), // any other case is probably bad
    }
}

fn with_schema(schema: JSONSchemaProps, crd: CustomResourceDefinition) -> CustomResourceDefinition {
    CustomResourceDefinition {
        spec: CustomResourceDefinitionSpec {
            validation: Some(CustomResourceValidation {
                open_api_v3_schema: Some(schema),
            }),
            ..crd.spec
        },
        ..crd
    }
}

#[cfg(test)]
mod tests {
    extern crate schemars;

    use schemars::schema_for;

    use super::*;

    #[test]
    fn print_schema() {
        let schema = schema_for!(NiFiDeploymentSpec);
        println!("{}", serde_json::to_string_pretty(&schema).unwrap());
    }
}
