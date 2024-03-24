use fusen_common::{server::Protocol, MethodResource};
use futures::{
    channel::{mpsc::Sender, oneshot::Receiver},
    SinkExt,
};
use http_body_util::Full;
use hyper::client::conn::http2::SendRequest;
use serde::{Deserialize, Serialize};
use std::{collections::HashMap, ops::Range, sync::Arc};
use tokio::sync::{
    mpsc::{self, UnboundedSender},
    oneshot, RwLock,
};
use tower::buffer::error;

use self::zookeeper::FusenZookeeper;
pub mod nacos;
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

    pub fn init(&self, map: Arc<RwLock<HashMap<String, Arc<Vec<Resource>>>>>) -> Box<dyn Register> {
        match self.register_type {
            RegisterType::ZooKeeper => {
                Box::new(FusenZookeeper::init(&self.addr, &self.name_space, map))
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
    pub version: Option<String>,
    pub methods: Vec<MethodResource>,
    pub ip: String,
    pub port: Option<String>,
}

impl Info {
    pub fn get_addr(&self) -> String {
        let mut ip = self.ip.clone();
        if let Some(port) = &self.port {
            ip.push(':');
            ip.push_str(port);
        }
        return ip;
    }
}

#[derive(Debug, Clone)]
pub struct SocketInfo {
    pub sender: Arc<RwLock<Option<SendRequest<Full<bytes::Bytes>>>>>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub enum Resource {
    Client(Info),
    Server(Info),
}

pub trait Register: Send + Sync {
    fn add_resource(&self, resource: Resource);

    fn check(&self, protocol: &Vec<Protocol>) -> crate::Result<String>;
}

#[allow(async_fn_in_trait)]
pub trait RegisterV2: Send + Sync {
    async fn register(&self, resource: Resource) -> Result<(), crate::Error>;

    async fn subscribe(&self, resource: Resource) -> Result<(), crate::Error>;
}

pub enum DirectorySender {
    GET,
    CHANGE(Vec<Resource>),
}

pub enum DirectoryReceiver {
    GET(Arc<Vec<Resource>>),
    CHANGE,
}

#[derive(Clone)]
pub struct Directory {
    sender: UnboundedSender<(DirectorySender, oneshot::Sender<DirectoryReceiver>)>,
}

impl Directory {
    pub async fn new() -> Self {
        let (s, mut r) =
            mpsc::unbounded_channel::<(DirectorySender, oneshot::Sender<DirectoryReceiver>)>();
        tokio::spawn(async move {
            let mut cache: Arc<Vec<Resource>> = Arc::new(vec![]);
            while let Some(msg) = r.recv().await {
                match msg.0 {
                    DirectorySender::GET => {
                        let _ = msg.1.send(DirectoryReceiver::GET(cache.clone()));
                    }
                    DirectorySender::CHANGE(resources) => {
                        cache = Arc::new(resources);
                        let _ = msg.1.send(DirectoryReceiver::CHANGE);
                    }
                }
            }
        });
        Self { sender: s }
    }

    pub async fn get(&mut self) -> Result<Arc<Vec<Resource>>, crate::Error> {
        let oneshot = oneshot::channel();
        let _ = self.sender.send((DirectorySender::GET, oneshot.0));
        let rev = oneshot.1.await.map_err(|e| e.to_string())?;
        match rev {
            DirectoryReceiver::GET(rev) => Ok(rev),
            DirectoryReceiver::CHANGE => Err("err receiver".into()),
        }
    }

    pub async fn change(&self, resource: Vec<Resource>) -> Result<(), crate::Error> {
        let oneshot = oneshot::channel();
        let _ = self
            .sender
            .send((DirectorySender::CHANGE(resource), oneshot.0));
        let rev = oneshot.1.await.map_err(|e| e.to_string())?;
        match rev {
            DirectoryReceiver::GET(_) => Err("err receiver".into()),
            DirectoryReceiver::CHANGE => Ok(()),
        }
    }
}
