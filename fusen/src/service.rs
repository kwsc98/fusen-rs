use crate::{
    Middleware, MiddlewareFuture, MiddlewareResult, RpcCategory, RpcContext, RpcError, RpcMessage,
    RpcRequest, RpcResponse, middleware::erase_middleware,
};
use fusen_contract::{MethodId, ServiceDescriptor};
use futures_util::FutureExt;
use serde::Serialize;
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

    /// Decodes the single typed request declared by an interface method.
    #[doc(hidden)]
    pub fn decode_request<T: RpcMessage>(&mut self) -> Result<RpcRequest<T>, RpcError> {
        let protocol = self.context.protocol();
        let arguments = self.context.take_arguments().unwrap_or_default();
        let body = crate::interface::decode_message(arguments, protocol)?;
        Ok(RpcRequest::from_server(body, &self.context))
    }

    /// Encodes a handler response without an unbudgeted JSON buffer.
    #[doc(hidden)]
    pub fn encode_response<T: Serialize>(self, response: RpcResponse<T>) -> MiddlewareResult {
        let (body, status, headers, extensions, attempts) = response.into_parts();
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
        let mut encoded = RpcResponse::success_with_budget(
            body,
            result_limit,
            envelope_bytes,
            &self.response_budget,
        )?;
        encoded.set_status(status)?;
        *encoded.headers_mut() = headers;
        *encoded.extensions_mut() = extensions;
        encoded.set_attempts(attempts);
        Ok(encoded)
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

/// Dispatch function emitted by the interface macro.
#[doc(hidden)]
pub type DispatchFn<T> = for<'a> fn(&'a T, ServerInvocation) -> MiddlewareFuture<'a>;

/// Fallible descriptor factory emitted by the interface macro.
#[doc(hidden)]
pub type DescriptorFn = fn() -> Result<&'static ServiceDescriptor, String>;

/// Framework-owned generated interface adapter.
#[doc(hidden)]
pub struct ServerService<T> {
    handler: T,
    descriptor: DescriptorFn,
    dispatch: DispatchFn<T>,
    head_middleware: Vec<Arc<dyn Middleware>>,
    middleware: Vec<Arc<dyn Middleware>>,
}

impl<T> ServerService<T> {
    /// Creates a generated interface adapter.
    #[doc(hidden)]
    pub fn new(handler: T, descriptor: DescriptorFn, dispatch: DispatchFn<T>) -> Self {
        Self {
            handler,
            descriptor,
            dispatch,
            head_middleware: Vec::new(),
            middleware: Vec::new(),
        }
    }

    /// Appends interface-local head middleware.
    #[doc(hidden)]
    pub fn head_middleware(mut self, middleware: impl Middleware) -> Self {
        self.head_middleware.push(erase_middleware(middleware));
        self
    }

    /// Appends interface-local decoded-call middleware.
    #[doc(hidden)]
    pub fn middleware(mut self, middleware: impl Middleware) -> Self {
        self.middleware.push(erase_middleware(middleware));
        self
    }
}

/// Conversion implemented by each generated `*Server` wrapper.
#[doc(hidden)]
pub trait IntoServerService: Sized {
    /// Erases the generated interface for server startup.
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
                handler: self.handler,
                dispatch: self.dispatch,
            }),
            head_middleware: self.head_middleware,
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
    fn call<'a>(&'a self, invocation: ServerInvocation) -> MiddlewareFuture<'a>;
}

struct FunctionDispatch<T> {
    handler: T,
    dispatch: DispatchFn<T>,
}

impl<T> ErasedDispatch for FunctionDispatch<T>
where
    T: Send + Sync + 'static,
{
    fn call<'a>(&'a self, invocation: ServerInvocation) -> MiddlewareFuture<'a> {
        let future = (self.dispatch)(&self.handler, invocation);
        Box::pin(async move {
            match AssertUnwindSafe(future).catch_unwind().await {
                Ok(result) => result,
                Err(_) => {
                    tracing::error!("RPC interface handler panicked");
                    Err(RpcError::framework(
                        RpcCategory::Internal,
                        "handler_panic",
                        "interface handler failed",
                    ))
                }
            }
        })
    }
}

/// Erased interface startup state consumed by [`Server`](crate::Server).
#[doc(hidden)]
pub struct PreparedService {
    descriptor: DescriptorFn,
    pub(crate) dispatch: Arc<dyn ErasedDispatch>,
    pub(crate) head_middleware: Vec<Arc<dyn Middleware>>,
    pub(crate) middleware: Vec<Arc<dyn Middleware>>,
}

impl PreparedService {
    pub(crate) fn descriptor(&self) -> Result<&'static ServiceDescriptor, String> {
        (self.descriptor)()
    }
}
