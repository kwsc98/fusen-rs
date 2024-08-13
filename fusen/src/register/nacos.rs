use super::{Category, Directory, Register};
use crate::register::Resource;
use fusen_common::FusenFuture;
use fusen_macro::url_config;
use nacos_sdk::api::{
    naming::{
        NamingChangeEvent, NamingEventListener, NamingService, NamingServiceBuilder,
        ServiceInstance,
    },
    props::ClientProps,
};
use std::{collections::HashMap, sync::Arc};
use tracing::{error, info};

#[derive(Clone)]
pub struct FusenNacos {
    application_name: String,
    naming_service: Arc<dyn NamingService + Sync + Send + 'static>,
    config: Arc<NacosConfig>,
}

#[url_config(attr = register)]
pub struct NacosConfig {
    server_addr: String,
    namespace: String,
    group: Option<String>,
    username: String,
    password: String,
}

impl FusenNacos {
    pub fn init(url: &str, application_name: String) -> crate::Result<Self> {
        let mut client_props = ClientProps::new();
        let config = NacosConfig::from_url(url)?;
        client_props = client_props
            .server_addr(config.server_addr.clone())
            .namespace(config.namespace.clone())
            .app_name(application_name.clone())
            .auth_username(config.username.clone())
            .auth_password(config.password.clone());
        let builder = NamingServiceBuilder::new(client_props);
        let builder = if !config.username.is_empty() {
            builder.enable_auth_plugin_http()
        } else {
            builder
        };
        let naming_service = Arc::new(builder.build()?);
        let config = Arc::new(config);
        let nacos = Self {
            application_name,
            naming_service: naming_service.clone(),
            config: config.clone(),
        };
        Ok(nacos)
    }
}

impl Register for FusenNacos {
    fn register(&self, resource: super::Resource) -> FusenFuture<Result<(), crate::Error>> {
        let nacos = self.clone();
        Box::pin(async move {
            let (nacos_service_name, group) = if let Category::Server = resource.category {
                (nacos.application_name.clone(), nacos.config.group.clone())
            } else {
                (get_service_name(&resource), resource.group.clone())
            };
            let nacos_service_instance =
                get_instance(resource.host, resource.port.unwrap(), resource.params);
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

    fn deregister(&self, resource: Resource) -> FusenFuture<Result<(), crate::Error>> {
        let nacos = self.clone();
        Box::pin(async move {
            let (nacos_service_name, group) = if let Category::Server = resource.category {
                (nacos.application_name.clone(), nacos.config.group.clone())
            } else {
                (get_service_name(&resource), resource.group.clone())
            };
            let nacos_service_instance =
                get_instance(resource.host, resource.port.unwrap(), resource.params);
            info!("deregister service: {}", nacos_service_name);
            let ret = nacos
                .naming_service
                .deregister_instance(nacos_service_name, group, nacos_service_instance)
                .await;
            if let Err(e) = ret {
                error!("deregister to nacos occur an error: {:?}", e);
                return Err(format!("deregister to nacos occur an error: {:?}", e).into());
            }
            Ok(())
        })
    }

    fn subscribe(&self, resource: super::Resource) -> FusenFuture<Result<Directory, crate::Error>> {
        let nacos = self.clone();
        Box::pin(async move {
            let nacos_service_name = if let Category::Server = &resource.category {
                get_application_name(&resource)
            } else {
                get_service_name(&resource)
            };
            info!("subscribe service: {}", nacos_service_name);
            let directory = Directory::new(resource.category).await;
            let directory_clone = directory.clone();
            let naming_service = nacos.naming_service.clone();
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
            let event_listener = ServiceChangeListener::new(directory);
            let event_listener = Arc::new(event_listener);
            naming_service
                .subscribe(
                    nacos_service_name,
                    resource.group,
                    Vec::new(),
                    event_listener,
                )
                .await?;
            Ok(directory_clone)
        })
    }
}

#[derive(Clone)]
struct ServiceChangeListener {
    directory: Directory,
}

impl ServiceChangeListener {
    fn new(directory: Directory) -> Self {
        Self { directory }
    }
}

impl NamingEventListener for ServiceChangeListener {
    fn event(&self, event: Arc<NamingChangeEvent>) {
        info!("service change: {}", event.service_name.clone());
        info!("nacos event: {:?}", event);
        let directory = self.directory.clone();
        let instances = event.instances.to_owned();
        tokio::spawn(async move {
            let instances = instances;
            let resources = if let Some(instances) = instances {
                to_resources(instances)
            } else {
                vec![]
            };
            let _ = directory.change(resources).await;
        });
    }
}

fn to_resources(service_instances: Vec<ServiceInstance>) -> Vec<Resource> {
    service_instances.iter().fold(vec![], |mut vec, e| {
        let resource = Resource {
            server_name: e.service_name().unwrap().to_string(),
            category: Category::Server,
            group: e.metadata().get("group").cloned(),
            version: e.metadata().get("version").cloned(),
            methods: vec![],
            host: e.ip().to_string(),
            port: Some(e.port().to_string()),
            weight: Some(e.weight),
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
        resource.version.as_ref().map_or("", |e| e),
        resource.group.as_ref().map_or("", |e| e),
    )
}

fn get_application_name(resource: &super::Resource) -> String {
    resource.server_name.clone()
}

fn get_instance(ip: String, port: String, params: HashMap<String, String>) -> ServiceInstance {
    nacos_sdk::api::naming::ServiceInstance {
        ip,
        port: port.parse().unwrap(),
        metadata: params,
        ..Default::default()
    }
}
