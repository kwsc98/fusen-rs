use crate::{
    Arguments, Middleware, RpcCategory, RpcContext, RpcError, RpcResponse, RpcResult,
    middleware::{MiddlewareDyn, erase_middleware},
    runtime::BoxFuture,
};
use fusen_contract::{MethodId, ServiceDescriptor};
use futures_util::FutureExt;
use serde::{Serialize, de::DeserializeOwned};
use serde_json::Value;
use std::{panic::AssertUnwindSafe, sync::Arc};

/// Decoded invocation passed to macro-generated server dispatch.
#[doc(hidden)]
pub struct ServerInvocation {
    context: RpcContext,
    max_response_body: usize,
    response_budget: Arc<crate::runtime::budget::ByteBudget>,
}

impl ServerInvocation {
    pub(crate) fn new(
        context: RpcContext,
        max_response_body: usize,
        response_budget: Arc<crate::runtime::budget::ByteBudget>,
    ) -> Self {
        Self {
            context,
            max_response_body,
            response_budget,
        }
    }

    /// Returns the declaration-order method identifier.
    pub const fn method_id(&self) -> MethodId {
        self.context.method().id()
    }

    /// Returns invocation metadata and arguments.
    pub const fn context(&self) -> &RpcContext {
        &self.context
    }

    /// Moves all decoded arguments into generated dispatch.
    pub fn take_arguments(&mut self) -> Arguments {
        std::mem::take(self.context.arguments_mut())
    }

    /// Returns the invocation context after generated argument decoding.
    pub fn into_context(self) -> RpcContext {
        self.context
    }

    /// Encodes one successful generated service result without a `Value` intermediate.
    pub fn encode_result<T>(self, value: T) -> RpcResult
    where
        T: Serialize,
    {
        let envelope_bytes = match self.context.protocol() {
            fusen_contract::WireProtocol::FusenV1 => 11,
            fusen_contract::WireProtocol::SpringCloudV1 => 0,
            _ => self.max_response_body,
        };
        let Some(result_limit) = self.max_response_body.checked_sub(envelope_bytes) else {
            return Err(RpcError::framework(
                RpcCategory::Internal,
                "response_too_large",
                "encoded RPC response exceeds the configured limit",
            ));
        };
        RpcResponse::success_with_budget(value, result_limit, envelope_bytes, &self.response_budget)
    }
}

/// Encodes one generated client argument.
#[doc(hidden)]
pub fn encode_argument<T>(value: &T) -> Result<Value, RpcError>
where
    T: Serialize + ?Sized,
{
    serde_json::to_value(value)
        .map_err(|error| RpcError::internal("failed to serialize RPC argument", error))
}

/// Decodes one generated server argument.
#[doc(hidden)]
pub fn decode_argument<T>(arguments: &mut Arguments, name: &str) -> Result<T, RpcError>
where
    T: DeserializeOwned,
{
    let value = arguments.remove(name).unwrap_or(Value::Null);
    if let Ok(decoded) = serde_json::from_value(value.clone()) {
        return Ok(decoded);
    }
    let converted = parse_spring_scalars(value);
    serde_json::from_value(converted).map_err(|error| {
        tracing::debug!(argument = name, ?error, "RPC argument decoding failed");
        RpcError::framework(
            RpcCategory::InvalidArgument,
            "invalid_argument",
            format!("invalid argument {name}"),
        )
    })
}

/// Rejects argument keys not declared by generated service metadata.
#[doc(hidden)]
pub fn finish_arguments(arguments: &Arguments) -> Result<(), RpcError> {
    if arguments.is_empty() {
        Ok(())
    } else {
        Err(RpcError::framework(
            RpcCategory::InvalidArgument,
            "unknown_argument",
            "request contains an unknown RPC argument",
        ))
    }
}

fn parse_spring_scalars(value: Value) -> Value {
    match value {
        Value::String(value) => serde_json::from_str(&value).unwrap_or(Value::String(value)),
        Value::Array(values) => {
            Value::Array(values.into_iter().map(parse_spring_scalars).collect())
        }
        value => value,
    }
}

/// Creates the stable dispatch error for an unknown declaration-order method ID.
#[doc(hidden)]
pub fn method_not_found(method: MethodId) -> RpcError {
    RpcError::framework(
        RpcCategory::Unimplemented,
        "method_not_found",
        format!("RPC method {} is not implemented", method.get()),
    )
}

/// Dispatch function emitted by the service macro.
#[doc(hidden)]
pub type DispatchFn<T> =
    for<'a> fn(&'a T, ServerInvocation) -> BoxFuture<'a, Result<RpcResponse, RpcError>>;

/// Framework-owned generated service adapter.
#[doc(hidden)]
pub struct ServerService<T> {
    service: T,
    descriptor: &'static ServiceDescriptor,
    dispatch: DispatchFn<T>,
    middleware: Vec<Arc<dyn MiddlewareDyn>>,
}

impl<T> ServerService<T> {
    /// Creates a generated service adapter.
    #[doc(hidden)]
    pub fn new(
        service: T,
        descriptor: &'static ServiceDescriptor,
        dispatch: DispatchFn<T>,
    ) -> Self {
        Self {
            service,
            descriptor,
            dispatch,
            middleware: Vec::new(),
        }
    }

    /// Appends service-local middleware.
    #[doc(hidden)]
    pub fn middleware(mut self, middleware: impl Middleware) -> Self {
        self.middleware.push(erase_middleware(middleware));
        self
    }
}

/// Conversion implemented by each generated `*Server` wrapper.
#[doc(hidden)]
pub trait IntoServerService: Sized {
    /// Erases the generated service for server startup.
    fn into_server_service(self) -> PreparedService;
}

impl<T> IntoServerService for ServerService<T>
where
    T: Send + Sync + 'static,
{
    fn into_server_service(self) -> PreparedService {
        PreparedService {
            descriptor: self.descriptor,
            dispatch: Arc::new(FunctionDispatch {
                service: self.service,
                dispatch: self.dispatch,
            }),
            middleware: self.middleware,
        }
    }
}

impl<T> ServerService<T>
where
    T: Send + Sync + 'static,
{
    /// Erases this generated adapter into server startup state.
    #[doc(hidden)]
    pub fn into_prepared(self) -> PreparedService {
        self.into_server_service()
    }
}

pub(crate) trait ErasedDispatch: Send + Sync {
    fn call<'a>(&'a self, invocation: ServerInvocation) -> BoxFuture<'a, RpcResult>;
}

struct FunctionDispatch<T> {
    service: T,
    dispatch: DispatchFn<T>,
}

impl<T> ErasedDispatch for FunctionDispatch<T>
where
    T: Send + Sync + 'static,
{
    fn call<'a>(&'a self, invocation: ServerInvocation) -> BoxFuture<'a, RpcResult> {
        let future = (self.dispatch)(&self.service, invocation);
        Box::pin(async move {
            match AssertUnwindSafe(future).catch_unwind().await {
                Ok(result) => result,
                Err(_) => {
                    tracing::error!("RPC service panicked");
                    Err(RpcError::framework(
                        RpcCategory::Internal,
                        "service_panic",
                        "service failed",
                    ))
                }
            }
        })
    }
}

/// Erased service startup state consumed by [`Server`](crate::Server).
#[doc(hidden)]
pub struct PreparedService {
    pub(crate) descriptor: &'static ServiceDescriptor,
    pub(crate) dispatch: Arc<dyn ErasedDispatch>,
    pub(crate) middleware: Vec<Arc<dyn MiddlewareDyn>>,
}

impl PreparedService {
    pub(crate) fn descriptor(&self) -> &'static ServiceDescriptor {
        self.descriptor
    }
}
