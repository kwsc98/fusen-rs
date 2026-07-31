extern crate self as fusen_rs;

use fusen_procedural_macro::SensitiveFields;

pub mod contract {
    include!("../support/sensitive_contract.rs");
}

#[derive(serde::Serialize, SensitiveFields)]
struct Flattened {
    #[sensitive(kind = "public")]
    username: String,
    #[serde(flatten)]
    #[sensitive(opaque)]
    extra: std::collections::BTreeMap<String, String>,
}

fn main() {}
