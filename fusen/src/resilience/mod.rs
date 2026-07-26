//! Retry and circuit-breaker policy foundations.

pub(crate) mod breaker;
pub(crate) mod retry;

pub use breaker::FailureClass;
pub use retry::{RetryDecision, RetryDecisionContext, RetryPolicy};
