#[macro_use]
extern crate serde_json;
extern crate futures;
extern crate k8s_openapi;
extern crate kube;
extern crate kube_derive;
extern crate serde;
extern crate anyhow;
#[macro_use]
extern crate log;

use k8s_openapi::Resource;
use kube::{Api, Client};
use crate::Namespace::*;

mod template;
mod handelbars_ext;
pub mod operator_config;
pub mod controller;
pub mod crd;

pub enum Namespace {
    All,
    SingleNamespace(String),
}


pub fn read_namespace() -> Namespace {
    let ns = std::env::var("NAMESPACE").unwrap_or("default".into());
    match ns.as_str() {
        "all" => Namespace::All,
        _ => Namespace::SingleNamespace(ns)
    }
}

pub fn get_api<T: Resource>(ns: &Namespace, client: Client) -> Api<T> {
    match ns {
        All => Api::all(client),
        SingleNamespace(name) => Api::namespaced(client, &name)
    }
}