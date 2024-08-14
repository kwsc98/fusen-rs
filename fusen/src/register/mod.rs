use self::nacos::FusenNacos;
use crate::protocol::socket::{InvokerAssets, Socket};
use fusen_common::{net::get_path, register::RegisterType, FusenFuture, MethodResource};
use fusen_procedural_macro::Data;
use rand::{distributions::WeightedIndex, prelude::Distribution, thread_rng};
use serde::{Deserialize, Serialize};
use std::{collections::HashMap, sync::Arc};
use tokio::sync::{
    mpsc::{self, UnboundedSender},
    oneshot,
};
pub mod nacos;

pub struct RegisterBuilder {
    register_type: RegisterType,
}

impl RegisterBuilder {
    pub fn new(config_url: String) -> crate::Result<Self> {
        let info: Vec<&str> = config_url.split("://").collect();
        if info[0] != "register" {
            return Err(format!("config url err is not register : {:?}", config_url).into());
        }
        let info: Vec<&str> = info[1].split('?').collect();
        let info = info[0].to_lowercase();
        let register_type = if info.contains("nacos") {
            RegisterType::Nacos(config_url)
        } else {
            return Err(format!("config url err : {:?}", config_url).into());
        };
        Ok(RegisterBuilder { register_type })
    }

    pub fn init(self, application_name: String) -> Box<dyn Register> {
        match self.register_type {
            RegisterType::Nacos(url) => Box::new(FusenNacos::init(&url, application_name).unwrap()),
        }
    }
}

#[derive(Serialize, Deserialize, Debug, Clone, Data, Default)]
pub struct Resource {
    server_name: String,
    category: Category,
    group: Option<String>,
    version: Option<String>,
    methods: Vec<MethodResource>,
    host: String,
    port: Option<String>,
    weight: Option<f64>,
    params: HashMap<String, String>,
}

impl Resource {
    pub fn get_addr(&self) -> String {
        let mut host = self.host.clone();
        if let Some(port) = &self.port {
            host.push(':');
            host.push_str(port);
        }
        host
    }
}

#[derive(Serialize, Deserialize, Debug, Clone, Default)]
pub enum Category {
    #[default]
    Client,
    Server,
    Service,
}

pub trait Register: Send + Sync {
    fn register(&self, resource: Resource) -> FusenFuture<Result<(), crate::Error>>;

    fn deregister(&self, resource: Resource) -> FusenFuture<Result<(), crate::Error>>;

    fn subscribe(&self, resource: Resource) -> FusenFuture<Result<Directory, crate::Error>>;
}

#[derive(Debug)]
pub enum DirectorySender {
    GET,
    CHANGE(Vec<Resource>),
}

pub enum DirectoryReceiver {
    GET(Arc<ResourceInfo>),
    CHANGE,
}

#[derive(Clone, Debug)]
pub struct Directory {
    sender: UnboundedSender<(DirectorySender, oneshot::Sender<DirectoryReceiver>)>,
}

#[derive(Debug, Data)]
pub struct ResourceInfo {
    dist: Option<WeightedIndex<f64>>,
    sockets: Vec<Arc<InvokerAssets>>,
}

impl ResourceInfo {
    pub fn new(sockets: Vec<Arc<InvokerAssets>>) -> Self {
        if sockets.is_empty() {
            Self {
                sockets,
                dist: None,
            }
        } else {
            let weights: Vec<f64> = sockets
                .iter()
                .map(|s| s.get_resource().get_weight().map_or(1_f64, |e| e))
                .collect();
            let dist = WeightedIndex::new(weights).unwrap();
            Self {
                sockets,
                dist: Some(dist),
            }
        }
    }

    pub fn select(&self) -> Option<Arc<InvokerAssets>> {
        self.dist.as_ref().map(|e| {
            self.sockets
                .get(e.sample(&mut thread_rng()))
                .cloned()
                .unwrap()
        })
    }
}

impl Directory {
    pub async fn new(category: Category) -> Self {
        let (s, mut r) =
            mpsc::unbounded_channel::<(DirectorySender, oneshot::Sender<DirectoryReceiver>)>();
        tokio::spawn(async move {
            let mut cache: Arc<ResourceInfo> = Arc::new(ResourceInfo::new(vec![]));
            while let Some(msg) = r.recv().await {
                match msg.0 {
                    DirectorySender::GET => {
                        let _ = msg.1.send(DirectoryReceiver::GET(cache.clone()));
                    }
                    DirectorySender::CHANGE(resources) => {
                        let map = cache
                            .get_sockets()
                            .iter()
                            .fold(HashMap::new(), |mut map, e| {
                                let key = get_path(
                                    e.get_resource().get_host().clone(),
                                    e.get_resource().get_port().as_deref(),
                                );
                                map.insert(
                                    format!("{}-{:?}", key, e.get_resource().get_weight()),
                                    e.clone(),
                                );
                                map
                            });
                        let mut res = vec![];
                        for item in resources {
                            let key = get_path(item.host.clone(), item.port.as_deref());
                            res.push(match map.get(&format!("{}-{:?}", key, item.weight)) {
                                Some(info) => info.clone(),
                                None => Arc::new(InvokerAssets::new(
                                    item,
                                    Socket::new(if let Category::Service = category {
                                        Some("http2")
                                    } else {
                                        None
                                    }),
                                )),
                            });
                        }
                        cache = Arc::new(ResourceInfo::new(res));
                        let _ = msg.1.send(DirectoryReceiver::CHANGE);
                    }
                }
            }
        });
        Self { sender: s }
    }

    pub async fn get(&self) -> Result<Arc<ResourceInfo>, crate::Error> {
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
