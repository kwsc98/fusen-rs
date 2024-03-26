use fusen_common::{server::Protocol, FusenFuture, MethodResource};
use futures::future::FusedFuture;
use http_body_util::Full;
use hyper::client::conn::http2::SendRequest;
use serde::{Deserialize, Serialize};
use std::{collections::HashMap, sync::Arc};
use tokio::sync::{
    mpsc::{self, UnboundedSender},
    oneshot, RwLock,
};

use self::nacos::FusenNacos;
pub mod nacos;
// pub mod zookeeper;

pub struct RegisterBuilder {
    register_type: RegisterType,
}

impl RegisterBuilder {
    pub fn new(register_type: RegisterType) -> Self {
        return RegisterBuilder { register_type };
    }

    pub fn init(&self) -> Box<dyn Register> {
        match &self.register_type {
            RegisterType::ZooKeeper(url) => {
                panic!("not support")
            }
            RegisterType::Nacos(url) => Box::new(FusenNacos::init(&url).unwrap()),
        }
    }
}

#[derive(Clone)]
pub enum RegisterType {
    ZooKeeper(String),
    Nacos(String),
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

#[derive(Debug, Clone)]
pub struct SocketInfo {
    pub sender: Arc<RwLock<Option<SendRequest<Full<bytes::Bytes>>>>>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub enum Category {
    Client,
    Server,
}

pub trait Register: Send + Sync {
    fn register(&self, resource: Resource) -> FusenFuture<Result<(), crate::Error>>;

    fn subscribe(&self, resource: Resource) -> FusenFuture<Result<Directory, crate::Error>>;

    fn check(&self, protocol: &Vec<Protocol>) -> crate::Result<String>;
}

#[derive(Debug)]
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
                println!("{:?}",msg.0);
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

    pub async fn get(&self) -> Result<Arc<Vec<Resource>>, crate::Error> {
        let oneshot = oneshot::channel();
        let _ = self.sender.clone().send((DirectorySender::GET, oneshot.0));
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
