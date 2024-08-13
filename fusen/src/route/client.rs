use crate::register::{Category, Directory, Register, Resource, ResourceInfo};
use async_recursion::async_recursion;
use fusen_common::FusenContext;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::mpsc::{self, UnboundedSender};
use tokio::sync::oneshot;

#[derive(Clone)]
pub struct Route {
    register: Option<Arc<Box<dyn Register>>>,
    sender: UnboundedSender<(RouteSender, oneshot::Sender<RouteReceiver>)>,
}

#[derive(Debug)]
pub enum RouteSender {
    GET(String),
    CHANGE((String, Directory)),
}

#[derive(Debug)]
pub enum RouteReceiver {
    GET(Option<Directory>),
    CHANGE,
}

impl Route {
    pub fn new(register: Option<Arc<Box<dyn Register>>>) -> Self {
        let (s, mut r) = mpsc::unbounded_channel::<(RouteSender, oneshot::Sender<RouteReceiver>)>();
        tokio::spawn(async move {
            let mut cache = HashMap::<String, Directory>::new();
            while let Some(msg) = r.recv().await {
                match msg.0 {
                    RouteSender::GET(key) => {
                        let _ = msg.1.send(RouteReceiver::GET(cache.get(&key).cloned()));
                    }
                    RouteSender::CHANGE(resources) => {
                        cache.insert(resources.0, resources.1);
                        let _ = msg.1.send(RouteReceiver::CHANGE);
                    }
                }
            }
        });
        Self {
            sender: s,
            register,
        }
    }

    #[async_recursion]
    pub async fn get_server_resource(
        &self,
        context: &FusenContext,
    ) -> crate::Result<Arc<ResourceInfo>> {
        let name = &context.context_info.class_name;
        let version = context.context_info.version.as_ref();
        let mut key = name.to_owned();
        key.push_str(&format!(":{:?}", context.server_type));
        if let Some(version) = version {
            key.push_str(&format!(":{}", version));
        }
        let oneshot = oneshot::channel();
        self.sender
            .send((RouteSender::GET(key.clone()), oneshot.0))?;
        let rev = oneshot.1.await.map_err(|e| e.to_string())?;
        match rev {
            RouteReceiver::GET(rev) => {
                if rev.is_none() {
                    let category = match context.server_type {
                        fusen_common::register::Type::Dubbo => Category::Service,
                        fusen_common::register::Type::SpringCloud => Category::Server,
                        fusen_common::register::Type::Fusen => Category::Service,
                        fusen_common::register::Type::Host(_) => Category::Server,
                    };
                    let resource_server = Resource {
                        server_name: name.to_string(),
                        category,
                        group: None,
                        version: version.map(|e| e.to_string()),
                        methods: vec![],
                        host: fusen_common::net::get_ip(),
                        port: None,
                        weight: None,
                        params: context.meta_data.clone_map(),
                    };
                    let directory =
                        if let fusen_common::register::Type::Host(host) = &context.server_type {
                            let directory = Directory::new(Category::Server).await;
                            let resource_server = Resource {
                                server_name: name.to_string(),
                                category: Category::Server,
                                group: None,
                                version: None,
                                methods: vec![],
                                host: host.clone(),
                                port: None,
                                weight: None,
                                params: HashMap::new(),
                            };
                            let _ = directory.change(vec![resource_server]).await;
                            directory
                        } else if let Some(register) = &self.register {
                            register.subscribe(resource_server).await?
                        } else {
                            return Err("must set register".into());
                        };
                    let oneshot = oneshot::channel();
                    self.sender
                        .send((RouteSender::CHANGE((key, directory.clone())), oneshot.0))?;
                    let rev = oneshot.1.await.map_err(|e| e.to_string())?;
                    return match rev {
                        RouteReceiver::GET(_) => Err("err receiver".into()),
                        RouteReceiver::CHANGE => Ok(directory.get().await?),
                    };
                }
                let rev = rev.unwrap();
                let info = rev.get().await?;
                return Ok(info);
            }
            RouteReceiver::CHANGE => return Err("err receiver".into()),
        }
    }
}
