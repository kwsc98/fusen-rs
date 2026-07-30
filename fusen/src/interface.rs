pub use crate::{
    context::{CallInfo, RpcArguments, RpcRequest, RpcResponse},
    rpc::{
        ErrorCode, InvalidErrorCode, RetryHint, RpcCategory, RpcError, RpcErrorDetails, RpcOrigin,
    },
};
use serde::{Serialize, de::DeserializeOwned};
use serde_json::Value;

/// Builds and validates one Spring Cloud method from an RPC message schema.
#[doc(hidden)]
pub fn spring_method<T: RpcMessage>(
    method: http::Method,
    path: &str,
) -> Result<fusen_contract::SpringCloudMethod, String> {
    let parameters = T::fields()
        .iter()
        .map(|field| {
            let source = match field.source {
                RpcFieldSource::Path => fusen_contract::SpringCloudParameterSource::Path,
                RpcFieldSource::Query => fusen_contract::SpringCloudParameterSource::Query,
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

/// The Spring Cloud wire role of one RPC message field.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum RpcFieldSource {
    /// A named path placeholder.
    Path,
    /// A URL query parameter.
    Query,
    /// The single JSON request body.
    Body,
}

/// Static wire schema for one named RPC message field.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RpcField {
    rust_name: &'static str,
    name: &'static str,
    source: RpcFieldSource,
    repeated: bool,
    parse_spring_json_primitive: bool,
}

impl RpcField {
    /// Creates field metadata used by `RpcMessage` derive output.
    #[doc(hidden)]
    pub const fn new(
        rust_name: &'static str,
        name: &'static str,
        source: RpcFieldSource,
        repeated: bool,
        parse_spring_json_primitive: bool,
    ) -> Self {
        Self {
            rust_name,
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

/// A named request DTO that can be represented by both supported unary JSON protocols.
pub trait RpcMessage: Serialize + DeserializeOwned + Send + 'static {
    /// Returns the derive-validated static field schema.
    fn fields() -> &'static [RpcField];
}

impl RpcMessage for () {
    fn fields() -> &'static [RpcField] {
        &[]
    }
}

#[doc(hidden)]
pub fn encode_message<T: RpcMessage>(message: &T) -> Result<RpcArguments, RpcError> {
    if T::fields().is_empty() {
        return match serde_json::to_value(message) {
            Ok(Value::Null) => Ok(RpcArguments::new()),
            Ok(Value::Object(values)) if values.is_empty() => Ok(RpcArguments::new()),
            Ok(_) => Err(invalid_message("RPC message must encode as a JSON object")),
            Err(error) => Err(RpcError::internal("failed to serialize RPC message", error)),
        };
    }
    let Value::Object(mut values) = serde_json::to_value(message)
        .map_err(|error| RpcError::internal("failed to serialize RPC message", error))?
    else {
        return Err(invalid_message("RPC message must encode as a JSON object"));
    };
    let mut arguments = RpcArguments::new();
    for field in T::fields() {
        let value = values.remove(field.rust_name).ok_or_else(|| {
            invalid_message(format!(
                "serialized RPC message is missing `{}`",
                field.rust_name
            ))
        })?;
        arguments.insert(field.name.to_owned(), value);
    }
    if !values.is_empty() {
        return Err(invalid_message(
            "serialized RPC message contains fields absent from its RpcMessage schema",
        ));
    }
    Ok(arguments)
}

#[doc(hidden)]
pub fn decode_message<T: RpcMessage>(
    mut arguments: RpcArguments,
    protocol: crate::WireProtocol,
) -> Result<T, RpcError> {
    if T::fields().is_empty() {
        if !arguments.is_empty() {
            return Err(unknown_argument());
        }
        return serde_json::from_value(Value::Null)
            .map_err(|error| RpcError::internal("failed to decode empty RPC message", error));
    }
    let mut values = serde_json::Map::new();
    for field in T::fields() {
        if let Some(mut value) = arguments.remove(field.name) {
            if protocol == crate::WireProtocol::SpringCloudV1
                && matches!(field.source, RpcFieldSource::Path | RpcFieldSource::Query)
                && field.parse_spring_json_primitive
            {
                value = parse_spring_json_primitives(value);
            }
            values.insert(field.rust_name.to_owned(), value);
        }
    }
    if !arguments.is_empty() {
        return Err(unknown_argument());
    }
    serde_json::from_value(Value::Object(values)).map_err(|error| {
        tracing::debug!(?error, "RPC message decoding failed");
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

fn invalid_message(message: impl Into<String>) -> RpcError {
    RpcError::framework(RpcCategory::InvalidArgument, "invalid_rpc_message", message)
}

fn unknown_argument() -> RpcError {
    RpcError::framework(
        RpcCategory::InvalidArgument,
        "unknown_argument",
        "request contains an unknown RPC argument",
    )
}
