use serde::{Deserialize, Serialize};

pub enum RegisterType {
    Nacos(String),
}

#[derive(Serialize, Deserialize, Debug, Clone, Default)]
pub enum Type {
    Dubbo,
    SpringCloud,
    #[default]
    Fusen,
    Host(String),
}
