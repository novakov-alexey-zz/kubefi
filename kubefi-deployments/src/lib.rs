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
pub mod operator_config;
pub mod controller;
pub mod crd;

pub enum Namespace {
    All,
    SingleNamespace(String),
}

pub fn get_api<T: Resource>(ns: &Namespace, client: Client) -> Api<T> {
    match ns {
        All => Api::all(client),
        SingleNamespace(name) => Api::namespaced(client, &name)
    }
}