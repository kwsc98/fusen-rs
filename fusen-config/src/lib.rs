#![warn(missing_docs)]
//! Static parsing and cancellation-safe, last-good hot configuration.

mod error;
mod hot;
mod static_config;

pub use error::{ConfigError, ConfigErrorKind, ConfigOperation};
pub use hot::{
    ConfigDocument, ConfigFormat, ConfigFuture, ConfigHandle, ConfigKey, ConfigKeyBuilder,
    ConfigSnapshot, ConfigSource, HotConfig,
};
pub use static_config::{load, parse, parse_toml, parse_yaml};

/// Provider lifecycle API for implementing [`ConfigSource`].
pub mod provider {
    pub use crate::hot::ConfigPublisher;
    pub use crate::hot::prepare_config as lifecycle;
}
