pub(crate) mod admission;
pub(crate) mod budget;
pub(crate) mod deadline;
pub(crate) mod metrics;

/// Sendable future used by the generated-code ABI and runtime-owned trait erasure.
#[doc(hidden)]
pub type BoxFuture<'a, T> = std::pin::Pin<Box<dyn std::future::Future<Output = T> + Send + 'a>>;
