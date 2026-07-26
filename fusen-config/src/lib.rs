#![warn(missing_docs)]
//! Static parsing and cancellation-safe, last-good hot configuration.

mod error;
mod hot;
mod static_config;

pub use error::{ConfigError, ConfigErrorKind, ConfigOperation};
pub use hot::{
    ConfigDocument, ConfigFormat, ConfigFuture, ConfigHandle, ConfigKey, ConfigKeyBuilder,
    ConfigPublisher, ConfigSnapshot, ConfigSource, HotConfig, prepare_config,
};
pub use static_config::{load, parse, parse_toml, parse_yaml};
