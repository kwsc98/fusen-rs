extern crate self as fusen_rs;

use fusen_procedural_macro::SensitiveFields;

pub mod contract {
    include!("../support/sensitive_contract.rs");
}

#[derive(SensitiveFields)]
#[sensitive(opaque = true)]
struct OpaqueWithValue;

#[derive(SensitiveFields)]
#[sensitive(kind = 7)]
struct NonStringKind;

#[derive(SensitiveFields)]
#[sensitive(bound = 7)]
struct NonStringBound;

#[derive(SensitiveFields)]
#[sensitive(bound = "T SensitiveFields")]
struct InvalidBound<T>(T);

#[derive(SensitiveFields)]
struct FieldBound {
    #[sensitive(bound = "String: contract::SensitiveFields")]
    value: String,
}

fn main() {}
