use serde::{Deserialize, Serialize};

use crate::url::UrlConfig;

pub enum RegisterType {
    ZooKeeper(Box<dyn UrlConfig>),
    Nacos(Box<dyn UrlConfig>),
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub enum Type {
    Dubbo,
    SpringCloud,
    Fusen,
}

impl Default for Type {
    fn default() -> Self {
        Type::Fusen
    }
}
