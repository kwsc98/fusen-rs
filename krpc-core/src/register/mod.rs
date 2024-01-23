use std::{collections::HashMap, sync::Arc};
use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;
pub mod zookeeper;

pub struct RegisterInfo {
    addr: String,
    name_space: String,
    register_type: RegisterType,
}

pub enum RegisterType {
    ZooKeeper,
    Nacos,
}
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Info {
    server_name: String,
    version: String,
    ip: String,
    port: Option<i32>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub enum Resource {
    Client(Info),
    Server(Info),
}

pub trait Register {
    async fn init(
        register_info: RegisterInfo,
        map: Arc<RwLock<HashMap<String, (Vec<Resource>, Vec<Resource>)>>>,
    ) -> Self;

    async fn add_resource(&mut self, resource: Resource);
}
