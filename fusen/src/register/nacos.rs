use super::{Category, Directory, Register, Type};
use crate::register::Resource;
use fusen_common::{net::get_ip, server::Protocol, url::UrlConfig, FusenFuture};
use fusen_macro::url_config;
use nacos_sdk::api::{
    naming::{
        NamingChangeEvent, NamingEventListener, NamingService, NamingServiceBuilder,
        ServiceInstance,
    },
    props::ClientProps,
};
use std::{collections::HashMap, sync::Arc};
use tokio::sync::mpsc::{self, UnboundedReceiver};
use tracing::{debug, error, info};

#[derive(Clone)]
pub struct FusenNacos {
    naming_service: Arc<dyn NamingService + Sync + Send + 'static>,
    config: Arc<NacosConfig>,
}

#[url_config(attr = register)]
pub struct NacosConfig {
    server_addr: String,
    namespace: String,
    group: Option<String>,
    app_name: Option<String>,
    username: String,
    password: String,
    server_type: Type,
}

impl FusenNacos {
    pub fn init(url: &str) -> crate::Result<Self> {
        let mut client_props = ClientProps::new();
        let config = NacosConfig::from_url(url)?;
        let app_name = config
            .app_name
            .as_ref()
            .map_or("fusen".to_owned(), |e| e.to_owned());
        client_props = client_props
            .server_addr(config.server_addr.clone())
            .namespace(config.namespace.clone())
            .app_name(app_name.clone())
            .auth_username(config.username.clone())
            .auth_password(config.password.clone());
        let builder = NamingServiceBuilder::new(client_props);
        let builder = if config.username.len() != 0 {
            builder.enable_auth_plugin_http()
        } else {
            builder
        };
        Ok(Self {
            naming_service: Arc::new(builder.build()?),
            config: Arc::new(config),
        })
    }
}

impl Register for FusenNacos {
    fn register(&self, resource: super::Resource) -> FusenFuture<Result<(), crate::Error>> {
        let nacos = self.clone();
        Box::pin(async move {
            if let Type::SpringCloud = nacos.config.server_type {
                return Ok(());
            }
            let group = (&resource).group.clone();
            let nacos_service_name = get_service_name(&resource);
            let nacos_service_instance =
                get_instance(resource.ip, resource.port.unwrap(), resource.params);
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
            let nacos_service_name = if let Type::SpringCloud = &nacos.config.server_type {
                get_application_name(&resource)
            } else {
                get_service_name(&resource)
            };
            info!("subscribe service: {}", nacos_service_name);
            let directory = Directory::new(Arc::new(nacos.config.server_type.clone())).await;
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

    fn check(&self, protocol: &Vec<Protocol>) -> FusenFuture<crate::Result<String>> {
        let nacos = self.clone();
        let protocol = protocol.clone();
        Box::pin(async move {
            let protocol = match &nacos.config.server_type {
                &Type::Dubbo => {
                    let Some(protocol) =
                        protocol.iter().find(|e| matches!(**e, Protocol::HTTP2(_)))
                    else {
                        return Err("Dubbo not find protocol for HTTP2".into());
                    };
                    (protocol, "DUBBO")
                }
                &Type::Fusen => {
                    let Some(protocol) =
                        protocol.iter().find(|e| matches!(**e, Protocol::HTTP2(_)))
                    else {
                        return Err("Fusen not find protocol for HTTP2".into());
                    };
                    (protocol, "FUSEN")
                }
                &Type::SpringCloud => {
                    let Some(protocol) = protocol.iter().find(|e| matches!(**e, Protocol::HTTP(_)))
                    else {
                        return Err("SpringCloud not find protocol for HTTP1".into());
                    };
                    (protocol, "SPRING_CLOUD")
                }
            };
            let port = match protocol.0 {
                Protocol::HTTP(port) => port,
                Protocol::HTTP2(port) => port,
            }
            .to_owned();
            let group = &nacos.config.group;
            let mut params = HashMap::new();
            params.insert(
                "preserved.register.source".to_owned(),
                protocol.1.to_owned(),
            );
            let app_name = nacos
                .config
                .as_ref()
                .app_name
                .as_ref()
                .map_or("fusen".to_owned(), |e| e.clone());
            let nacos_instance = get_instance(get_ip(), port.clone(), params);
            info!("register application: {}", app_name);
            let ret = nacos
                .naming_service
                .register_instance(app_name.to_string(), group.to_owned(), nacos_instance)
                .await;
            if let Err(e) = ret {
                error!("register to nacos occur an error: {:?}", e);
                return Err(format!("register to nacos occur an error: {:?}", e).into());
            }
            return Ok(port);
        })
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

fn get_application_name(resource: &super::Resource) -> String {
    match resource.params.get("spring_cloud_name") {
        Some(name) => name.clone(),
        None => resource.server_name.clone(),
    }
}

fn get_instance(ip: String, port: String, params: HashMap<String, String>) -> ServiceInstance {
    nacos_sdk::api::naming::ServiceInstance {
        ip,
        port: port.parse().unwrap(),
        metadata: params,
        ..Default::default()
    }
}
