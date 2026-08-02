use crate::{Error, ErrorCategory};
use serde::{Serialize, de::DeserializeOwned};
use serde_json::Value;

/// Builds and validates one HTTP mapping from generated parameter metadata.
#[doc(hidden)]
pub fn http_method(
    method: http::Method,
    path: &str,
    consumes: &str,
    produces: &str,
    fields: &[ArgumentField],
) -> Result<fusen_contract::HttpOperation, String> {
    let parameters = fields
        .iter()
        .map(|field| {
            let source = match field.source {
                ArgumentSource::Path => fusen_contract::HttpParameterSource::Path,
                ArgumentSource::Query => fusen_contract::HttpParameterSource::Query,
                ArgumentSource::Header => fusen_contract::HttpParameterSource::Header,
                ArgumentSource::Cookie => fusen_contract::HttpParameterSource::Cookie,
                ArgumentSource::BodyField => fusen_contract::HttpParameterSource::BodyField,
                ArgumentSource::Body => fusen_contract::HttpParameterSource::Body,
                ArgumentSource::QueryMap => fusen_contract::HttpParameterSource::QueryMap,
                ArgumentSource::HeaderMap => fusen_contract::HttpParameterSource::HeaderMap,
            };
            let cardinality = if field.repeated {
                fusen_contract::HttpParameterCardinality::Repeated
            } else {
                fusen_contract::HttpParameterCardinality::Scalar
            };
            fusen_contract::HttpParameter::new(field.name, source, cardinality)
                .map_err(|error| error.to_string())
        })
        .collect::<Result<Vec<_>, _>>()?;
    fusen_contract::HttpOperation::new(method, path, parameters, consumes, produces)
        .map_err(|error| error.to_string())
}

/// The HTTP wire role of one interface parameter.
#[doc(hidden)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum ArgumentSource {
    /// A named path placeholder.
    Path,
    /// A URL query parameter.
    Query,
    /// A named HTTP header.
    Header,
    /// A named request cookie.
    Cookie,
    /// A named field in the synthesized JSON request body object.
    BodyField,
    /// The single JSON request body.
    Body,
    /// An object expanded into query parameters.
    QueryMap,
    /// An object expanded into request headers.
    HeaderMap,
}

/// Static wire metadata for one named interface parameter.
#[doc(hidden)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ArgumentField {
    name: &'static str,
    source: ArgumentSource,
    repeated: bool,
}

impl ArgumentField {
    /// Creates field metadata generated from an interface parameter.
    #[doc(hidden)]
    pub const fn new(name: &'static str, source: ArgumentSource, repeated: bool) -> Self {
        Self {
            name,
            source,
            repeated,
        }
    }
}

#[doc(hidden)]
pub fn encode_argument<T: Serialize>(value: &T) -> Result<Value, Error> {
    serde_json::to_value(value)
        .map_err(|error| Error::internal("failed to serialize invocation argument", error))
}

pub(crate) fn decode_argument<T: DeserializeOwned>(
    value: Value,
    text_encoded: bool,
) -> Result<T, Error> {
    if let Ok(decoded) = serde_json::from_value(value.clone()) {
        return Ok(decoded);
    }
    if text_encoded && let Ok(decoded) = serde_json::from_value(parse_spring_json_scalars(value)) {
        return Ok(decoded);
    }
    tracing::debug!("service invocation message decoding failed");
    Err(Error::framework(
        ErrorCategory::InvalidArgument,
        "invalid_argument",
        "request does not match the invocation message schema",
    ))
}

fn parse_spring_json_scalars(value: Value) -> Value {
    match value {
        Value::String(value) => match serde_json::from_str(&value) {
            Ok(parsed @ (Value::Null | Value::Bool(_) | Value::Number(_))) => parsed,
            _ => Value::String(value),
        },
        Value::Array(values) => {
            Value::Array(values.into_iter().map(parse_spring_json_scalars).collect())
        }
        value => value,
    }
}

pub(crate) fn unknown_argument() -> Error {
    Error::framework(
        ErrorCategory::InvalidArgument,
        "unknown_argument",
        "request contains an unknown invocation argument",
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::Deserialize;
    use serde_json::json;

    type Flag = bool;

    #[derive(Debug, Deserialize, PartialEq)]
    #[serde(transparent)]
    struct Count(u64);

    #[test]
    fn http_text_decoding_prefers_the_original_string_shape() {
        let text = decode_argument::<String>(json!("true"), true).unwrap();
        assert_eq!(text, "true");

        let flag = decode_argument::<Flag>(json!("true"), true).unwrap();
        assert!(flag);

        let count = decode_argument::<Count>(json!("42"), true).unwrap();
        assert_eq!(count, Count(42));

        let values = decode_argument::<Vec<u64>>(json!(["1", "2"]), true).unwrap();
        assert_eq!(values, [1, 2]);
    }

    #[test]
    fn http_body_decoding_does_not_reinterpret_json_strings() {
        let error = decode_argument::<u64>(json!("42"), false).unwrap_err();
        assert_eq!(error.code().as_str(), "invalid_argument");
    }
}
