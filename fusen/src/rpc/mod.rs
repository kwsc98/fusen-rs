//! Stable request, response, and error types used by RPC services and middleware.

mod error;

pub use error::{ErrorCode, InvalidErrorCode, ProblemDetails, RpcCategory, RpcError, RpcOrigin};
