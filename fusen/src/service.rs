use crate::{
    Arguments, Call, Context, Error, ErrorCategory, Interceptor, InterceptorFuture,
    InterceptorResult, Response, interceptor::erase_interceptor,
};
use fusen_contract::{MethodId, ServiceDescriptor};
use futures_util::FutureExt;
use serde::Serialize;
use std::{panic::AssertUnwindSafe, sync::Arc};

/// Decoded invocation passed to macro-generated server dispatch.
#[doc(hidden)]
pub struct ServerInvocation {
    context: Context,
    arguments: Arguments,
    max_response_body: usize,
    response_budget: Arc<crate::runtime::budget::ByteBudget>,
}

impl ServerInvocation {
    pub(crate) fn new(
        mut context: Context,
        max_response_body: usize,
        response_budget: Arc<crate::runtime::budget::ByteBudget>,
    ) -> Self {
        let arguments = context.take_arguments().unwrap_or_default();
        Self {
            context,
            arguments,
            max_response_body,
            response_budget,
        }
    }

    /// Returns the declaration-order method identifier.
    pub const fn method_id(&self) -> MethodId {
        self.context.method().id()
    }

    /// Returns server-bound call metadata for an explicit `#[param(context)]` parameter.
    #[doc(hidden)]
    pub fn call(&self) -> Call {
        Call::from_server(&self.context)
    }

    /// Removes and decodes one named method argument.
    #[doc(hidden)]
    pub fn decode_argument<T: serde::de::DeserializeOwned>(
        &mut self,
        name: &str,
        text_encoded: bool,
    ) -> Result<T, Error> {
        let value = self
            .arguments
            .remove(name)
            .unwrap_or(serde_json::Value::Null);
        crate::interface::decode_argument(value, text_encoded)
    }

    /// Rejects arguments that are absent from the generated method schema.
    #[doc(hidden)]
    pub fn finish_arguments(&self) -> Result<(), Error> {
        if self.arguments.is_empty() {
            Ok(())
        } else {
            Err(crate::interface::unknown_argument())
        }
    }

    /// Encodes a handler response without an unbudgeted JSON buffer.
    #[doc(hidden)]
    pub fn encode_response<T: Serialize>(self, response: Response<T>) -> InterceptorResult {
        let (body, status, headers, extensions, attempts) = response.into_parts();
        let mut encoded =
            Response::success_with_budget(body, self.max_response_body, 0, &self.response_budget)?;
        encoded.mark_declared_serialize_schema_origin(self.context.method());
        encoded.set_status(status)?;
        *encoded.headers_mut() = headers;
        *encoded.extensions_mut() = extensions;
        encoded.set_attempts(attempts);
        Ok(encoded)
    }
}

/// Creates the stable dispatch error for an unknown declaration-order method ID.
#[doc(hidden)]
pub fn method_not_found(method: MethodId) -> Error {
    Error::framework(
        ErrorCategory::Unimplemented,
        "method_not_found",
        format!("service method {} is not implemented", method.get()),
    )
}

/// Dispatch function emitted by the interface macro.
#[doc(hidden)]
pub type DispatchFn<T> = for<'a> fn(&'a T, ServerInvocation) -> InterceptorFuture<'a>;

/// Fallible descriptor factory emitted by the interface macro.
#[doc(hidden)]
pub type DescriptorFn = fn() -> Result<&'static ServiceDescriptor, String>;

/// Framework-owned generated interface adapter.
#[doc(hidden)]
pub struct ServerService<T> {
    handler: T,
    descriptor: DescriptorFn,
    dispatch: DispatchFn<T>,
    head_interceptor: Vec<Arc<dyn Interceptor>>,
    interceptor: Vec<Arc<dyn Interceptor>>,
}

impl<T> ServerService<T> {
    /// Creates a generated interface adapter.
    #[doc(hidden)]
    pub fn new(handler: T, descriptor: DescriptorFn, dispatch: DispatchFn<T>) -> Self {
        Self {
            handler,
            descriptor,
            dispatch,
            head_interceptor: Vec::new(),
            interceptor: Vec::new(),
        }
    }

    /// Appends interface-local head interceptor.
    #[doc(hidden)]
    pub fn head_interceptor(mut self, interceptor: impl Interceptor) -> Self {
        self.head_interceptor.push(erase_interceptor(interceptor));
        self
    }

    /// Appends interface-local decoded-call interceptor.
    #[doc(hidden)]
    pub fn interceptor(mut self, interceptor: impl Interceptor) -> Self {
        self.interceptor.push(erase_interceptor(interceptor));
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
            head_interceptor: self.head_interceptor,
            interceptor: self.interceptor,
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
    fn call<'a>(&'a self, invocation: ServerInvocation) -> InterceptorFuture<'a>;
}

struct FunctionDispatch<T> {
    handler: T,
    dispatch: DispatchFn<T>,
}

impl<T> ErasedDispatch for FunctionDispatch<T>
where
    T: Send + Sync + 'static,
{
    fn call<'a>(&'a self, invocation: ServerInvocation) -> InterceptorFuture<'a> {
        let future = (self.dispatch)(&self.handler, invocation);
        Box::pin(async move {
            match AssertUnwindSafe(future).catch_unwind().await {
                Ok(result) => result,
                Err(_) => {
                    tracing::error!("service interface handler panicked");
                    Err(Error::framework(
                        ErrorCategory::Internal,
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
    pub(crate) head_interceptor: Vec<Arc<dyn Interceptor>>,
    pub(crate) interceptor: Vec<Arc<dyn Interceptor>>,
}

impl PreparedService {
    pub(crate) fn descriptor(&self) -> Result<&'static ServiceDescriptor, String> {
        (self.descriptor)()
    }
}
