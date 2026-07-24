#![warn(missing_docs)]
//! Nacos service registration, discovery, and hot configuration integration.

use serde::{Deserialize, Serialize};

mod config;
mod register;

pub use config::NacosConfiguration;
pub use register::NacosRegister;

/// Nacos client connection and authentication settings.
#[derive(Serialize, Deserialize, Clone, Default)]
pub struct NacosConfig {
    /// Nacos server addresses.
    pub server_addr: String,
    /// Optional namespace identifier.
    pub namespace: Option<String>,
    /// Optional authentication username.
    pub username: Option<String>,
    /// Optional authentication password.
    pub password: Option<String>,
}

impl std::fmt::Debug for NacosConfig {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("NacosConfig")
            .field("server_addr", &self.server_addr)
            .field("namespace", &self.namespace)
            .field("username", &self.username)
            .field("password", &self.password.as_ref().map(|_| "***"))
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn debug_output_redacts_password() {
        let config = NacosConfig {
            password: Some("secret".into()),
            ..Default::default()
        };
        let output = format!("{config:?}");
        assert!(output.contains("***"));
        assert!(!output.contains("secret"));
    }
}
