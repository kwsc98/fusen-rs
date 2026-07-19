#![warn(missing_docs)]
//! Dependency-light shared types used across fusen-rs crates.

/// Supported wire protocol identifiers.
pub mod protocol;
#[allow(missing_docs)]
pub mod resource;
#[allow(missing_docs)]
pub mod utils;

/// Owned, sendable future used by object-safe async contracts.
pub type BoxFuture<T> = std::pin::Pin<Box<dyn std::future::Future<Output = T> + Send>>;
/// Borrowing, sendable future used by middleware contracts.
pub type BoxFutureV2<'a, T> = std::pin::Pin<Box<dyn std::future::Future<Output = T> + Send + 'a>>;

/// JSON types re-exported for generated code.
pub use serde_json;
