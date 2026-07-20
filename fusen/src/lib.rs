#![warn(missing_docs)]
#![doc = include_str!("../../README.md")]
//! Reliable JSON RPC runtime for HTTP/1.1 and HTTP/2 services.

/// Client configuration, endpoint selection, and invocation runtime.
pub mod client;
/// Stable framework errors and RFC 9457 response types.
pub mod error;
#[allow(missing_docs)]
pub mod filter;
#[allow(missing_docs)]
pub mod handler;
#[allow(missing_docs)]
pub mod protocol;
/// Transactional server builder and runtime.
pub mod server;

/// Shared protocol and resource types used by generated code.
pub use fusen_internal_common;
/// Attribute macros used to generate RPC clients, services, and handlers.
pub use fusen_procedural_macro;
/// HTTP types used by generated service metadata.
pub use http;
