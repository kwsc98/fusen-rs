extern crate self as fusen_rs;

use fusen_procedural_macro::SensitiveFields;

pub mod contract {
    include!("../support/sensitive_contract.rs");
}

#[derive(SensitiveFields)]
#[sensitive(kind = "contains spaces")]
struct InvalidKind;

fn main() {}
