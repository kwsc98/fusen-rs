use serde::{Deserialize, Serialize};

pub enum RegisterType {
    ZooKeeper(String),
    Nacos(String),
}

#[derive(Serialize, Deserialize, Debug, Clone, Default)]
pub enum Type {
    Dubbo,
    SpringCloud,
    #[default]
    Fusen,
}

impl Type {
    pub fn get_name(&self) -> String {
        match &self {
            Type::Dubbo => "DUBBO",
            Type::SpringCloud => "SPRING_CLOUD",
            Type::Fusen => "FUSEN",
        }
        .to_string()
    }
}
