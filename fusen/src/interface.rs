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
    parse_spring_json_primitive: bool,
}

impl RpcField {
    /// Creates field metadata generated from an interface parameter.
    #[doc(hidden)]
    pub const fn new(
        name: &'static str,
        source: RpcFieldSource,
        repeated: bool,
        parse_spring_json_primitive: bool,
    ) -> Self {
        Self {
            name,
            source,
            repeated,
            parse_spring_json_primitive,
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
    mut value: Value,
    protocol: crate::WireProtocol,
    parse_spring_json_primitive: bool,
) -> Result<T, RpcError> {
    if protocol == crate::WireProtocol::SpringCloudV1 && parse_spring_json_primitive {
        value = parse_spring_json_primitives(value);
    }
    serde_json::from_value(value).map_err(|_| {
        tracing::debug!("RPC message decoding failed");
        RpcError::framework(
            RpcCategory::InvalidArgument,
            "invalid_argument",
            "request does not match the RPC message schema",
        )
    })
}

fn parse_spring_json_primitives(value: Value) -> Value {
    match value {
        Value::String(value) => serde_json::from_str(&value).unwrap_or(Value::String(value)),
        Value::Array(values) => Value::Array(
            values
                .into_iter()
                .map(parse_spring_json_primitives)
                .collect(),
        ),
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
