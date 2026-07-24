#![warn(missing_docs)]
//! Typed file configuration and latest-wins hot configuration updates.

mod config;
mod error;
mod string;

pub use config::{
    ConfigManager, ConfigResponse, HotConfigChangeListener, config_build, get_config_by_path,
    get_toml_by_context, get_yaml_by_context,
};
pub use error::Error;
pub use fusen_config_macro::StrategyDebug;

/// Framework internals used by configuration providers and generated code.
#[doc(hidden)]
pub mod __private {
    pub use crate::config::{ConfigCloseFuture, ConfigLifecycle};
    pub use crate::string::{limit_str, mask_str};
}
