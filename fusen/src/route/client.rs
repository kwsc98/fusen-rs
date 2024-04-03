use crate::register::{Category, Directory, Register, Resource, ResourceInfo};
use async_recursion::async_recursion;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::mpsc::{self, UnboundedSender};
use tokio::sync::oneshot;

#[derive(Clone)]
pub struct Route {
    register: Arc<Box<dyn Register>>,
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
    pub fn new(register: Box<dyn Register>) -> Self {
        let (s, mut r) = mpsc::unbounded_channel::<(RouteSender, oneshot::Sender<RouteReceiver>)>();
        tokio::spawn(async move {
            let mut cache = HashMap::<String, Directory>::new();
            while let Some(msg) = r.recv().await {
                match msg.0 {
                    RouteSender::GET(key) => {
                        let _ = msg
                            .1
                            .send(RouteReceiver::GET(cache.get(&key).map(|e| e.clone())));
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
            register: Arc::new(register),
        }
    }

    #[async_recursion]
    pub async fn get_server_resource(
        &self,
        name: &str,
        version: std::option::Option<&'async_recursion str>,
    ) -> crate::Result<ResourceInfo> {
        let mut key = name.to_owned();
        if let Some(version) = version {
            key.push_str(":");
            key.push_str(version);
        }
        let oneshot = oneshot::channel();
        let _ = self
            .sender
            .send((RouteSender::GET(key.clone()), oneshot.0))?;
        let rev = oneshot.1.await.map_err(|e| e.to_string())?;
        match rev {
            RouteReceiver::GET(rev) => {
                if let None = rev {
                    let resource_client = Resource {
                        server_name: name.to_string(),
                        category: Category::Client,
                        group: None,
                        version: version.map(|e| e.to_string()),
                        methods: vec![],
                        ip: fusen_common::net::get_ip(),
                        port: None,
                        params: HashMap::new(),
                    };
                    let directory = self.register.subscribe(resource_client).await;
                    if let Err(err) = directory {
                        return Err(err);
                    }
                    let directory = directory.unwrap();
                    let oneshot = oneshot::channel();
                    let _ = self
                        .sender
                        .send((RouteSender::CHANGE((key, directory)), oneshot.0))?;
                    let rev = oneshot.1.await.map_err(|e| e.to_string())?;
                    match rev {
                        RouteReceiver::GET(_) => return Err("err receiver".into()),
                        RouteReceiver::CHANGE => {}
                    };
                    return self.get_server_resource(name, version).await;
                }
                let rev = rev.unwrap();
                let info = rev.get().await?;
                return Ok(info);
            }
            RouteReceiver::CHANGE => return Err("err receiver".into()),
        }
    }
}
