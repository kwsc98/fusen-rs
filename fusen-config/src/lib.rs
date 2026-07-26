#![warn(missing_docs)]
//! Static parsing and cancellation-safe, last-good hot configuration.

mod error;
mod hot;
mod static_config;

pub use error::{ConfigError, ConfigErrorKind, ConfigOperation};
pub use hot::{
    ConfigDocument, ConfigFormat, ConfigFuture, ConfigHandle, ConfigKey, ConfigKeyBuilder,
    ConfigSnapshot, HotConfig,
};
pub use static_config::{load, parse, parse_toml, parse_yaml};

/// Private ABI for provider adapters maintained in this workspace.
#[doc(hidden)]
pub mod __adapter {
    pub use crate::hot::{ConfigPublisher, prepare_config};
}
