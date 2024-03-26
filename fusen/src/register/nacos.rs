use fusen_common::{server::Protocol, url::UrlConfig, FusenFuture};
use fusen_macro::UrlConfig;
use nacos_sdk::api::{
    naming::{
        NamingChangeEvent, NamingEventListener, NamingService, NamingServiceBuilder,
        ServiceInstance,
    },
    props::ClientProps,
};
use serde::{Deserialize, Serialize};
use std::{pin::Pin, sync::Arc};
use tokio::sync::mpsc::{self, UnboundedReceiver};
use tracing::{debug, error, info};

use crate::register::Resource;

use super::{Category, Directory, Register};

#[derive(Clone)]
pub struct FusenNacos {
    naming_service: Arc<dyn NamingService + Sync + Send + 'static>,
}

#[derive(UrlConfig, Serialize, Deserialize)]
pub struct NacosConfig {
    server_addr: String,
    namespace: Option<String>,
    app_name: Option<String>,
    username: String,
    password: String,
}

impl NacosConfig {
    pub fn new(server_addr: &str, username: &str, password: &str) -> Self {
        Self {
            server_addr: server_addr.to_string(),
            namespace: None,
            app_name: None,
            username: username.to_string(),
            password: password.to_string(),
        }
    }
}

impl FusenNacos {
    pub fn init(url: &str) -> crate::Result<Self> {
        let mut client_props = ClientProps::new();
        let config = NacosConfig::from_url(url)?;
        let namespace = config.namespace.map_or("public".to_owned(), |e| e);
        let app_name = config.app_name.map_or("fusen".to_owned(), |e| e);
        client_props = client_props
            .server_addr(config.server_addr)
            .namespace(namespace)
            .app_name(app_name)
            .auth_username(String::default())
            .auth_password(String::default());
        let naming_service = NamingServiceBuilder::new(client_props).build()?;
        Ok(Self {
            naming_service: Arc::new(naming_service),
        })
    }
}

impl Register for FusenNacos {
    fn register(&self, resource: super::Resource) -> FusenFuture<Result<(), crate::Error>> {
        let nacos = self.clone(); 
        println!("1aaa");

        Box::pin(async move {
            println!("dsdasdsadsad");
            let group = (&resource).group.clone();
            let nacos_service_name = get_service_name(&resource);
            let nacos_service_instance = get_service_instance(resource);
            info!("register service: {}", nacos_service_name);
            let ret = nacos
                .naming_service
                .register_instance(nacos_service_name, group, nacos_service_instance)
                .await;
            if let Err(e) = ret {
                error!("register to nacos occur an error: {:?}", e);
                return Err(format!("register to nacos occur an error: {:?}", e).into());
            }
            Ok(())
        })
    }

    fn subscribe(&self, resource: super::Resource) -> FusenFuture<Result<Directory, crate::Error>> {
        let nacos = self.clone(); 
        Box::pin(async move {
            let nacos_service_name = get_service_name(&resource);
            info!("subscribe service: {}", nacos_service_name);
            let directory = Directory::new().await;
            let directory_clone = directory.clone();
            let naming_service = nacos.naming_service.clone();
            let (event_listener, receiver) = ServiceChangeListener::new();
            let event_listener = Arc::new(event_listener);
            let service_instances = naming_service
                .get_all_instances(
                    nacos_service_name.clone(),
                    resource.group.clone(),
                    Vec::new(),
                    false,
                )
                .await?;
            let service_instances = to_resources(service_instances);
            directory.change(service_instances).await?;
            let _ = naming_service
                .subscribe(
                    nacos_service_name.clone(),
                    resource.group,
                    Vec::new(),
                    event_listener,
                )
                .await?;
            tokio::spawn(async move {
                let mut receiver = receiver;
                let directory = directory;
                while let Some(change) = receiver.recv().await {
                    let resources = to_resources(change);
                    let _ = directory.change(resources).await;
                }
            });
            Ok(directory_clone)
        })
    }

    fn check(&self, protocol: &Vec<Protocol>) -> crate::Result<String> {
        for protocol in protocol {
            if let Protocol::HTTP2(port) = protocol {
                return Ok(port.clone());
            }
        }
        return Err("need monitor Http2".into());
    }
}

struct ServiceChangeListener {
    tx: mpsc::UnboundedSender<Vec<ServiceInstance>>,
}

impl ServiceChangeListener {
    fn new() -> (Self, UnboundedReceiver<Vec<ServiceInstance>>) {
        let mpsc = mpsc::unbounded_channel();
        (Self { tx: mpsc.0 }, mpsc.1)
    }
    async fn changed(&self, instances: Vec<ServiceInstance>) -> Result<(), crate::Error> {
        self.tx.send(instances).map_err(|e| e.into())
    }
}

impl NamingEventListener for ServiceChangeListener {
    fn event(&self, event: Arc<NamingChangeEvent>) {
        debug!("service change {}", event.service_name.clone());
        debug!("nacos event: {:?}", event);

        let instances = event.instances.as_ref();
        match instances {
            None => {
                let _ = self.changed(Vec::default());
            }
            Some(instances) => {
                let _ = self.changed(instances.clone());
            }
        }
    }
}

fn to_resources(service_instances: Vec<ServiceInstance>) -> Vec<Resource> {
    service_instances.iter().fold(vec![], |mut vec, e| {
        let resource = Resource {
            server_name: e.service_name().unwrap().to_string(),
            category: Category::Server,
            group: e.metadata().get("group").map(|e| e.clone()),
            version: e.metadata().get("version").map(|e| e.clone()),
            methods: vec![],
            ip: e.ip().to_string(),
            port: Some(e.port().to_string()),
            params: e.metadata().clone(),
        };
        vec.push(resource);
        vec
    })
}

fn get_service_name(resource: &super::Resource) -> String {
    let category = "providers";
    format!(
        "{}:{}:{}:{}",
        category,
        resource.server_name,
        resource.version.as_ref().map_or("", |e| &e),
        resource.group.as_ref().map_or("", |e| &e),
    )
}

fn get_service_instance(resource: super::Resource) -> ServiceInstance {
    let ip = resource.ip;
    let port = resource.port;
    nacos_sdk::api::naming::ServiceInstance {
        ip,
        port: port.unwrap().parse().unwrap(),
        metadata: resource.params,
        ..Default::default()
    }
}
