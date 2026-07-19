#![warn(missing_docs)]
//! Common configuration, logging, Nacos, and utility integrations.

#[allow(missing_docs)]
pub mod config;
#[allow(missing_docs)]
pub mod date;
#[allow(missing_docs)]
pub mod error;
#[allow(missing_docs)]
pub mod log;
#[allow(missing_docs)]
pub mod nacos;
#[allow(missing_docs)]
pub mod string;
#[allow(missing_docs)]
pub mod utils;

/// Attribute parsing support for common macros.
pub use fusen_common_derive_macro;
/// Derive macros provided by fusen-common.
pub use fusen_common_procedural_macro;
/// Serde derive support used by configuration consumers.
pub use serde::Deserialize;
