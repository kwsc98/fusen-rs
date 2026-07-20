#![warn(missing_docs)]
//! Common configuration, logging, Nacos, and utility integrations.

#[cfg(feature = "config")]
#[allow(missing_docs)]
pub mod config;
#[cfg(feature = "logging")]
#[allow(missing_docs)]
pub mod date;
#[allow(missing_docs)]
pub mod error;
#[cfg(feature = "logging")]
#[allow(missing_docs)]
pub mod log;
#[cfg(feature = "nacos")]
#[allow(missing_docs)]
pub mod nacos;
#[allow(missing_docs)]
pub mod string;
#[allow(missing_docs)]
#[cfg(feature = "async-utils")]
pub mod utils;

/// Attribute parsing support for common macros.
pub use fusen_common_derive_macro;
/// Derive macros provided by fusen-common.
pub use fusen_common_procedural_macro;
/// Serde derive support used by configuration consumers.
pub use serde::Deserialize;
