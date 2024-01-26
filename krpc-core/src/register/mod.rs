use self::zookeeper::KrpcZookeeper;
use http_body_util::Full;
use hyper::client::conn::http2::SendRequest;
use serde::{Deserialize, Serialize};
use std::{collections::HashMap, sync::Arc};
use tokio::sync::RwLock;
pub mod zookeeper;

pub struct RegisterBuilder {
    addr: String,
    name_space: String,
    register_type: RegisterType,
}

impl RegisterBuilder {
    pub fn new(addr: &str, name_space: &str, register_type: RegisterType) -> Self {
        return RegisterBuilder {
            addr: addr.to_string(),
            name_space: name_space.to_string(),
            register_type,
        };
    }

    pub fn init(&self, map: Arc<RwLock<HashMap<String, Vec<SocketInfo>>>>) -> Box<dyn Register> {
        match self.register_type {
            RegisterType::ZooKeeper => {
                Box::new(KrpcZookeeper::init(&self.addr, &self.name_space, map))
            }
            RegisterType::Nacos => panic!("not support"),
        }
    }
}

#[derive(Clone)]
pub enum RegisterType {
    ZooKeeper,
    Nacos,
}
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Info {
    pub server_name: String,
    pub version: String,
    pub ip: String,
    pub port: Option<String>,
}

impl Info {
    pub fn get_addr(&self) -> String {
        self.ip.clone() + ":" + &self.port.clone().unwrap()
    }
}

#[derive(Debug, Clone)]
pub struct SocketInfo {
    pub info: Info,
    pub sender: Arc<RwLock<Option<SendRequest<Full<bytes::Bytes>>>>>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub enum Resource {
    Client(Info),
    Server(Info),
}

pub trait Register: Send + Sync {
    fn add_resource(&self, resource: Resource);
}
