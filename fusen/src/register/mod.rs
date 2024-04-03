use bytes::Bytes;
use fusen_common::{net::get_path, server::Protocol, url::UrlConfig, FusenFuture, MethodResource};
use hyper::client::conn::http2::SendRequest;

use serde::{Deserialize, Serialize};
use std::{collections::HashMap, sync::Arc};
use tokio::sync::{
    mpsc::{self, UnboundedSender},
    oneshot, RwLock,
};

use crate::StreamBody;

use self::{nacos::FusenNacos, zookeeper::FusenZookeeper};
pub mod nacos;
pub mod zookeeper;

pub struct RegisterBuilder {
    register_type: RegisterType,
}

impl RegisterBuilder {
    pub fn new(config: Box<dyn UrlConfig>) -> crate::Result<Self> {
        let config_url = config.to_url()?;
        let info: Vec<&str> = config_url.split("://").collect();
        if info[0] != "register" {
            return Err(format!("config url err is not register : {:?}", config_url).into());
        }
        let info: Vec<&str> = info[1].split("?").collect();
        let info = info[0].to_lowercase();
        let register_type = if info.contains("nacos") {
            RegisterType::Nacos(config)
        } else if info.contains("zookeeper") {
            RegisterType::ZooKeeper(config)
        } else {
            return Err(format!("config url err : {:?}", config_url).into());
        };
        return Ok(RegisterBuilder { register_type });
    }

    pub fn init(self) -> Box<dyn Register> {
        match self.register_type {
            RegisterType::ZooKeeper(url) => {
                Box::new(FusenZookeeper::init(&url.to_url().unwrap()).unwrap())
            }
            RegisterType::Nacos(url) => Box::new(FusenNacos::init(&url.to_url().unwrap()).unwrap()),
        }
    }
}

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

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Resource {
    pub server_name: String,
    pub category: Category,
    pub group: Option<String>,
    pub version: Option<String>,
    pub methods: Vec<MethodResource>,
    pub ip: String,
    pub port: Option<String>,
    pub params: HashMap<String, String>,
}

impl Resource {
    pub fn get_addr(&self) -> String {
        let mut ip = self.ip.clone();
        if let Some(port) = &self.port {
            ip.push(':');
            ip.push_str(port);
        }
        return ip;
    }
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub enum Category {
    Client,
    Server,
}

pub trait Register: Send + Sync {
    fn register(&self, resource: Resource) -> FusenFuture<Result<(), crate::Error>>;

    fn subscribe(&self, resource: Resource) -> FusenFuture<Result<Directory, crate::Error>>;

    fn check(&self, protocol: &Vec<Protocol>) -> FusenFuture<crate::Result<String>>;
}

#[derive(Debug)]
pub enum DirectorySender {
    GET,
    CHANGE(Vec<Resource>),
}

pub enum DirectoryReceiver {
    GET(Vec<Arc<SocketInfo>>),
    CHANGE,
}

#[derive(Clone, Debug)]
pub struct Directory {
    server_type: Arc<Type>,
    sender: UnboundedSender<(DirectorySender, oneshot::Sender<DirectoryReceiver>)>,
}

pub struct ResourceInfo {
    pub server_type: Arc<Type>,
    pub info: Vec<Arc<SocketInfo>>,
}

pub struct SocketInfo {
    pub resource: Resource,
    pub socket: SocketType,
}

pub enum SocketType {
    HTTP1,
    HTTP2(RwLock<Option<SendRequest<StreamBody<Bytes, hyper::Error>>>>),
}

impl Directory {
    pub async fn new(server_type: Arc<Type>) -> Self {
        let (s, mut r) =
            mpsc::unbounded_channel::<(DirectorySender, oneshot::Sender<DirectoryReceiver>)>();
        let server_type_clone = server_type.clone();
        tokio::spawn(async move {
            let mut cache: Vec<Arc<SocketInfo>> = vec![];
            while let Some(msg) = r.recv().await {
                match msg.0 {
                    DirectorySender::GET => {
                        let _ = msg.1.send(DirectoryReceiver::GET(cache.clone()));
                    }
                    DirectorySender::CHANGE(resources) => {
                        let map = cache.iter().fold(HashMap::new(), |mut map, e| {
                            let key = get_path(e.resource.ip.clone(), e.resource.port.as_deref());
                            map.insert(key, e.clone());
                            map
                        });
                        let mut res = vec![];
                        for item in resources {
                            let key = get_path(item.ip.clone(), item.port.as_deref());
                            res.push(match map.get(&key) {
                                Some(info) => info.clone(),
                                None => Arc::new(SocketInfo {
                                    resource: item,
                                    socket: match server_type_clone.as_ref() {
                                        Type::Dubbo => SocketType::HTTP2(RwLock::new(None)),
                                        Type::SpringCloud => SocketType::HTTP1,
                                        Type::Fusen => SocketType::HTTP2(RwLock::new(None)),
                                    },
                                }),
                            });
                        }
                        cache = res;
                        let _ = msg.1.send(DirectoryReceiver::CHANGE);
                    }
                }
            }
        });
        Self {
            sender: s,
            server_type,
        }
    }

    pub async fn get(&self) -> Result<ResourceInfo, crate::Error> {
        let oneshot = oneshot::channel();
        let _ = self.sender.send((DirectorySender::GET, oneshot.0));
        let rev = oneshot.1.await.map_err(|e| e.to_string())?;
        match rev {
            DirectoryReceiver::GET(rev) => Ok(ResourceInfo {
                server_type: self.server_type.clone(),
                info: rev,
            }),
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
