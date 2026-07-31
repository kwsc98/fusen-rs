extern crate self as fusen_rs;

use fusen_procedural_macro::SensitiveFields;

pub mod contract {
    include!("../support/sensitive_contract.rs");
}

#[derive(SensitiveFields)]
#[sensitive(kind = "secret", opaque)]
struct ConflictingType;

#[derive(serde::Serialize, SensitiveFields)]
struct ConflictingField {
    #[sensitive(kind = "phone", opaque)]
    phone: String,
}

fn main() {}
