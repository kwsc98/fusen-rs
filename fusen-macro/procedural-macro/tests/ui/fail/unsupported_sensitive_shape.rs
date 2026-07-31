extern crate self as fusen_rs;

use fusen_procedural_macro::SensitiveFields;

pub mod contract {
    include!("../support/sensitive_contract.rs");
}

#[derive(serde::Serialize, SensitiveFields)]
struct Tuple(String);

#[derive(serde::Serialize, SensitiveFields)]
enum Choice {
    One,
    Two(String),
}

#[derive(serde::Serialize, SensitiveFields)]
struct Flattened {
    #[serde(flatten)]
    values: Nested,
}

#[derive(serde::Serialize, SensitiveFields)]
struct CustomSerialized {
    #[serde(serialize_with = "serialize_value")]
    value: Nested,
}

#[derive(serde::Serialize, serde::Deserialize, SensitiveFields)]
#[serde(tag = "kind")]
struct Tagged {
    value: String,
}

#[derive(serde::Serialize)]
struct Nested;

fn serialize_value<S>(_: &Nested, serializer: S) -> Result<S::Ok, S::Error>
where
    S: serde::Serializer,
{
    serializer.serialize_unit()
}

fn main() {}
