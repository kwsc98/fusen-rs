#![warn(missing_docs)]
//! Nacos adapters for cancellation-safe service discovery and hot configuration.

mod config;
mod register;

pub use config::NacosConfigSource;
pub use register::NacosRegistry;

fn client_props(
    config: &NacosConfig,
    application_name: &str,
) -> nacos_sdk::api::props::ClientProps {
    let props = nacos_sdk::api::props::ClientProps::new()
        .env_first(false)
        .server_addr(config.server_addr().to_owned())
        .namespace(config.namespace().unwrap_or_default().to_owned())
        .app_name(application_name.to_owned());
    match (config.username(), config.password()) {
        (Some(username), Some(password)) => props
            .auth_username(username.to_owned())
            .auth_password(password.to_owned()),
        _ => props,
    }
}

fn validate_application_name(value: &str) -> Result<(), &'static str> {
    if value.is_empty()
        || value.len() > 128
        || value.trim() != value
        || value.bytes().any(|byte| byte.is_ascii_control())
    {
        Err(
            "Nacos application name must be 1-128 bytes without surrounding whitespace or control characters",
        )
    } else {
        Ok(())
    }
}

/// Nacos client connection and authentication settings.
#[derive(Clone, serde::Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct NacosConfig {
    server_addr: String,
    namespace: Option<String>,
    username: Option<String>,
    password: Option<String>,
}

impl NacosConfig {
    /// Starts a builder with production defaults.
    pub fn builder() -> NacosConfigBuilder {
        NacosConfigBuilder::default()
    }

    /// Returns the comma-separated Nacos server addresses.
    pub fn server_addr(&self) -> &str {
        &self.server_addr
    }

    /// Returns the optional Nacos namespace.
    pub fn namespace(&self) -> Option<&str> {
        self.namespace.as_deref()
    }

    /// Returns the optional HTTP authentication username.
    pub fn username(&self) -> Option<&str> {
        self.username.as_deref()
    }

    /// Returns the optional HTTP authentication password.
    ///
    /// Callers must not include this value in logs, metrics, or error messages.
    pub fn password(&self) -> Option<&str> {
        self.password.as_deref()
    }

    fn validate(&self) -> Result<(), &'static str> {
        if self.server_addr.trim().is_empty() {
            return Err("Nacos server address must not be empty");
        }
        if self.server_addr.trim() != self.server_addr {
            return Err("Nacos server address must not contain surrounding whitespace");
        }
        if self.username.is_some() != self.password.is_some() {
            return Err("Nacos username and password must be configured together");
        }
        Ok(())
    }
}

impl Default for NacosConfig {
    fn default() -> Self {
        Self {
            server_addr: "127.0.0.1:8848".into(),
            namespace: None,
            username: None,
            password: None,
        }
    }
}

impl std::fmt::Debug for NacosConfig {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("NacosConfig")
            .field("server_addr", &self.server_addr)
            .field("namespace", &self.namespace)
            .field("username", &self.username)
            .field("password", &self.password.as_ref().map(|_| "<redacted>"))
            .finish()
    }
}

/// Builder for [`NacosConfig`].
#[derive(Clone, Debug, Default)]
pub struct NacosConfigBuilder {
    config: NacosConfig,
}

impl NacosConfigBuilder {
    /// Sets the comma-separated Nacos server addresses.
    pub fn server_addr(mut self, server_addr: impl Into<String>) -> Self {
        self.config.server_addr = server_addr.into();
        self
    }

    /// Sets the Nacos namespace.
    pub fn namespace(mut self, namespace: impl Into<String>) -> Self {
        self.config.namespace = Some(namespace.into());
        self
    }

    /// Sets HTTP authentication credentials.
    pub fn credentials(mut self, username: impl Into<String>, password: impl Into<String>) -> Self {
        self.config.username = Some(username.into());
        self.config.password = Some(password.into());
        self
    }

    /// Builds the immutable configuration.
    ///
    /// Provider-specific validation occurs before either adapter performs network I/O.
    pub fn build(self) -> NacosConfig {
        self.config
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builder_fields_are_private_but_observable_through_getters() {
        let config = NacosConfig::builder()
            .server_addr("nacos.internal:8848")
            .namespace("prod")
            .credentials("service", "secret")
            .build();
        assert_eq!(config.server_addr(), "nacos.internal:8848");
        assert_eq!(config.namespace(), Some("prod"));
        assert_eq!(config.username(), Some("service"));
        assert_eq!(config.password(), Some("secret"));
    }

    #[test]
    fn debug_output_redacts_passwords() {
        let config = NacosConfig::builder()
            .credentials("service", "do-not-log")
            .build();
        let debug = format!("{config:?}");
        assert!(debug.contains("<redacted>"));
        assert!(!debug.contains("do-not-log"));
    }

    #[test]
    fn credentials_must_be_configured_as_a_pair() {
        let config = NacosConfig {
            username: Some("service".into()),
            ..NacosConfig::default()
        };
        assert!(config.validate().is_err());
    }
}
