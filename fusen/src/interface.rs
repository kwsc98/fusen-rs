pub use crate::{
    context::{CallInfo, RpcArguments, RpcCall, RpcResponse},
    rpc::{
        ErrorCode, InvalidErrorCode, RetryHint, RpcCategory, RpcError, RpcErrorDetails, RpcOrigin,
    },
};
use serde::{Serialize, de::DeserializeOwned};
use serde_json::Value;

/// Builds and validates one HTTP mapping from generated parameter metadata.
#[doc(hidden)]
pub fn http_method(
    method: http::Method,
    path: &str,
    fields: &[RpcField],
) -> Result<fusen_contract::SpringCloudMethod, String> {
    let parameters = fields
        .iter()
        .map(|field| {
            let source = match field.source {
                RpcFieldSource::Path => fusen_contract::SpringCloudParameterSource::Path,
                RpcFieldSource::Query => fusen_contract::SpringCloudParameterSource::Query,
                RpcFieldSource::BodyField => fusen_contract::SpringCloudParameterSource::BodyField,
                RpcFieldSource::Body => fusen_contract::SpringCloudParameterSource::Body,
            };
            let cardinality = if field.repeated {
                fusen_contract::SpringCloudParameterCardinality::Repeated
            } else {
                fusen_contract::SpringCloudParameterCardinality::Scalar
            };
            fusen_contract::SpringCloudParameter::new(field.name, source, cardinality)
                .map_err(|error| error.to_string())
        })
        .collect::<Result<Vec<_>, _>>()?;
    fusen_contract::SpringCloudMethod::new(method, path, parameters)
        .map_err(|error| error.to_string())
}

/// The HTTP wire role of one interface parameter.
#[doc(hidden)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum RpcFieldSource {
    /// A named path placeholder.
    Path,
    /// A URL query parameter.
    Query,
    /// A named field in the synthesized JSON request body object.
    BodyField,
    /// The single JSON request body.
    Body,
}

/// Static wire metadata for one named interface parameter.
#[doc(hidden)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RpcField {
    name: &'static str,
    source: RpcFieldSource,
    repeated: bool,
}

impl RpcField {
    /// Creates field metadata generated from an interface parameter.
    #[doc(hidden)]
    pub const fn new(name: &'static str, source: RpcFieldSource, repeated: bool) -> Self {
        Self {
            name,
            source,
            repeated,
        }
    }

    /// Returns the field's wire name.
    pub const fn name(&self) -> &'static str {
        self.name
    }

    /// Returns the field's Spring Cloud wire role.
    pub const fn source(&self) -> RpcFieldSource {
        self.source
    }

    /// Returns whether a query field uses repeated keys.
    pub const fn is_repeated(&self) -> bool {
        self.repeated
    }
}

#[doc(hidden)]
pub fn encode_argument<T: Serialize>(value: &T) -> Result<Value, RpcError> {
    serde_json::to_value(value)
        .map_err(|error| RpcError::internal("failed to serialize RPC argument", error))
}

#[doc(hidden)]
pub fn decode_argument<T: DeserializeOwned>(
    value: Value,
    protocol: crate::WireProtocol,
    spring_text: bool,
) -> Result<T, RpcError> {
    if let Ok(decoded) = serde_json::from_value(value.clone()) {
        return Ok(decoded);
    }
    if protocol == crate::WireProtocol::SpringCloudV1
        && spring_text
        && let Ok(decoded) = serde_json::from_value(parse_spring_json_scalars(value))
    {
        return Ok(decoded);
    }
    tracing::debug!("RPC message decoding failed");
    Err(RpcError::framework(
        RpcCategory::InvalidArgument,
        "invalid_argument",
        "request does not match the RPC message schema",
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

pub(crate) fn unknown_argument() -> RpcError {
    RpcError::framework(
        RpcCategory::InvalidArgument,
        "unknown_argument",
        "request contains an unknown RPC argument",
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
    fn spring_text_decoding_prefers_the_original_string_shape() {
        let text =
            decode_argument::<String>(json!("true"), crate::WireProtocol::SpringCloudV1, true)
                .unwrap();
        assert_eq!(text, "true");

        let flag = decode_argument::<Flag>(json!("true"), crate::WireProtocol::SpringCloudV1, true)
            .unwrap();
        assert!(flag);

        let count = decode_argument::<Count>(json!("42"), crate::WireProtocol::SpringCloudV1, true)
            .unwrap();
        assert_eq!(count, Count(42));

        let values = decode_argument::<Vec<u64>>(
            json!(["1", "2"]),
            crate::WireProtocol::SpringCloudV1,
            true,
        )
        .unwrap();
        assert_eq!(values, [1, 2]);
    }

    #[test]
    fn spring_body_decoding_does_not_reinterpret_json_strings() {
        let error = decode_argument::<u64>(json!("42"), crate::WireProtocol::SpringCloudV1, false)
            .unwrap_err();
        assert_eq!(error.code().as_str(), "invalid_argument");
    }
}
