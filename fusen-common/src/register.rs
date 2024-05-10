use serde::{Deserialize, Serialize};

use crate::url::UrlConfig;

pub enum RegisterType {
    ZooKeeper(Box<dyn UrlConfig>),
    Nacos(Box<dyn UrlConfig>),
}

#[derive(Serialize, Deserialize, Debug, Clone, Default)]
pub enum Type {
    Dubbo,
    SpringCloud,
    #[default]
    Fusen,
}
