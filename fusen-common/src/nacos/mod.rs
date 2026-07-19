use serde::{Deserialize, Serialize};

pub mod config;
pub mod register;

#[derive(Serialize, Deserialize, Debug, Clone, Default)]
pub struct NacosConfig {
    pub server_addr: String,
    pub namespace: Option<String>,
    pub username: Option<String>,
    pub password: Option<String>,
}
