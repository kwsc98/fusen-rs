extern crate self as fusen_rs;

use fusen_procedural_macro::SensitiveFields;

pub mod contract {
    include!("../support/sensitive_contract.rs");
}

#[derive(serde::Serialize, SensitiveFields)]
struct Request {
    value: Missing,
}

#[derive(serde::Serialize)]
struct Missing;

fn main() {}
