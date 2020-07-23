use std::env;

use anyhow::{Error, Result};

pub struct IngressConfig {
    pub host: String,
    pub ingress_class: String,
}

pub struct AuthLdapConfig {
    pub host: String,
    pub search_base: String,
    pub search_filter: String,
}

pub struct SiteToSite {
    pub secure: Boolean,
    pub port: u16,
}

pub struct Properties {
    pub provenance_storage: String,
    pub site_to_site: SiteToSite,
    pub web_proxy_host: String,
    pub http_port: u16,
    pub https_port: u16,
    pub need_client_auth: Boolean,
    pub authorizer: String,
    pub cluster_secure: Boolean,
    pub is_node: Boolean,
    pub cluster_port: u16,
}

pub struct Config {
    pub image: Option<String>,
    pub zk_image: Option<String>,
    pub storage_class: Option<String>,
    pub ingress: Option<IngressConfig>,
    pub auth_ldap_config: Option<AuthLdapConfig>,
    pub props: Properties,
}

pub fn read_config() -> Result<Config> {
    let image = env::var("IMAGE_NAME").ok();
    let zk_image = env::var("ZK_IMAGE_NAME").ok();
    let storage_class = env::var("STORAGE_CLASS").ok();
    let ingress_class = env::var("INGRESS_CLASS").ok();
    let ingress_host = env::var("INGRESS_HOST").ok();
    let ingress = match (ingress_class, ingress_host) {
        (Some(c), Some(h)) => Ok(Some(IngressConfig { host: h, ingress_class: c })),
        (Some(_), None) | (None, Some(_)) => Err(Error::msg("INGRESS_CLASS or INGRESS_HOST is not specified")),
        (None, None) => Ok(None)
    }?;
    Ok(Config { image, zk_image, storage_class, ingress })
}

