use serde::{Deserialize, Serialize};

pub mod config;
pub mod register;

#[derive(Serialize, Deserialize, Clone, Default)]
pub struct NacosConfig {
    pub server_addr: String,
    pub namespace: Option<String>,
    pub username: Option<String>,
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
