extern crate self as fusen_rs;

use fusen_procedural_macro::SensitiveFields;

pub mod contract {
    include!("../support/sensitive_contract.rs");
}

#[derive(serde::Serialize, serde::Deserialize, SensitiveFields)]
#[serde(transparent)]
struct DirectionalTransparent {
    #[serde(skip_deserializing)]
    serialize_only: String,
    #[serde(skip_serializing)]
    deserialize_only: String,
}

fn main() {}
