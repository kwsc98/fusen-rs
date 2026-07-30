//! Stable request, response, and error types used by RPC services and middleware.

mod error;

pub(crate) use error::ProblemDetails;
pub use error::{
    ErrorCode, InvalidErrorCode, RetryHint, RpcCategory, RpcError, RpcErrorDetails, RpcOrigin,
};
