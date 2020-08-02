extern crate anyhow;
extern crate futures;
extern crate k8s_openapi;
extern crate kube;
extern crate kube_derive;
#[macro_use]
extern crate log;
extern crate serde;
#[macro_use]
extern crate serde_json;

use k8s_openapi::Resource;
use kube::{Api, Client};

use crate::Namespace::*;

pub mod config;
pub mod controller;
pub mod crd;
mod handelbars_ext;
pub mod template;

pub enum Namespace {
    All,
    SingleNamespace(String),
}

pub fn read_namespace() -> Namespace {
    let ns = std::env::var("NAMESPACE").unwrap_or_else(|_| "default".into());
    match ns.as_str() {
        "all" => Namespace::All,
        _ => Namespace::SingleNamespace(ns),
    }
}

pub fn get_api<T: Resource>(ns: &Namespace, client: Client) -> Api<T> {
    match ns {
        All => Api::all(client),
        SingleNamespace(name) => Api::namespaced(client, &name),
    }
}

pub fn read_type<T>(default: &'static str) -> &'static str {
    std::any::type_name::<T>()
        .split("::")
        .last()
        .unwrap_or_else(|| default)
}
