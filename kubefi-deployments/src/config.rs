use std::path::PathBuf;

use anyhow::{Error, Result};
use hocon::{Hocon, HoconLoader};
use serde::Deserialize;
use serde_json::{Number, Value};
use std::fmt::Debug;

#[derive(Deserialize, Debug)]
pub struct KubefiConfig {
    pub crd_schema_path: PathBuf,
    pub replace_existing_crd: bool,
}

pub fn read_kubefi_config() -> Result<KubefiConfig, Error> {
    let cfg: KubefiConfig = HoconLoader::new()
        .load_file("./conf/kubefi.conf")?
        .resolve()?;
    Ok(cfg)
}

pub fn read_nifi_config() -> Result<Value> {
    debug!("Loading nifi config...");
    let hocon = HoconLoader::new().load_file("./conf/nifi.conf")?.hocon()?;
    hocon_to_json(hocon).ok_or_else(|| Error::msg("Failed to convert config file to JSON"))
}

fn hocon_to_json(hocon: Hocon) -> Option<Value> {
    match hocon {
        Hocon::Boolean(b) => Some(Value::Bool(b)),
        Hocon::Integer(i) => Some(Value::Number(Number::from(i))),
        Hocon::Real(f) => Some(Value::Number(
            Number::from_f64(f).unwrap_or_else(|| Number::from(0)),
        )),
        Hocon::String(s) => Some(Value::String(s)),
        Hocon::Array(vec) => Some(Value::Array(
            vec.into_iter()
                .map(hocon_to_json)
                .filter_map(|i| i)
                .collect(),
        )),
        Hocon::Hash(map) => Some(Value::Object(
            map.into_iter()
                .map(|(k, v)| (k, hocon_to_json(v)))
                .filter_map(|(k, v)| v.map(|v| (k, v)))
                .collect(),
        )),
        Hocon::Null => Some(Value::Null),
        Hocon::BadValue(_) => None,
    }
}
