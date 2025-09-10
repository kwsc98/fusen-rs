use crate::{Register, directory::Directory, error::RegisterError};
use fusen_internal_common::{BoxFuture, protocol::Protocol, resource::service::ServiceResource};
use nacos_sdk::api::{
    naming::{
        NamingChangeEvent, NamingEventListener, NamingService, NamingServiceBuilder,
        ServiceInstance,
    },
    props::ClientProps,
};
use std::sync::Arc;
use tracing::{error, info};

#[derive(Clone)]
pub struct NacosRegister {
    naming_service: Arc<NamingService>,
    _config: Arc<NacosConfig>,
    group: Option<String>,
}

#[derive(Default)]
pub struct NacosConfig {
    pub application_name: String,
    pub server_addr: String,
    pub namespace: Option<String>,
    pub username: Option<String>,
    pub password: Option<String>,
}

impl NacosRegister {
    pub fn init(config: NacosConfig, group: Option<String>) -> Result<Self, RegisterError> {
        let mut client_props = ClientProps::new();
        client_props = client_props
            .server_addr(config.server_addr.clone())
            .namespace(config.namespace.clone().unwrap_or_default())
            .app_name(config.application_name.clone())
            .auth_username(config.username.clone().unwrap_or_default())
            .auth_password(config.password.clone().unwrap_or_default());
        let builder = NamingServiceBuilder::new(client_props);
        let builder = if config.username.is_some() {
            builder.enable_auth_plugin_http()
        } else {
            builder
        };
        let naming_service = Arc::new(
            builder
                .build()
                .map_err(|e| RegisterError::Error(Box::new(e)))?,
        );
        let nacos = Self {
            naming_service: naming_service.clone(),
            _config: Arc::new(config),
            group,
        };
        Ok(nacos)
    }
}

impl Register for NacosRegister {
    fn register(
        &self,
        resource: Arc<ServiceResource>,
        protocol: Protocol,
    ) -> BoxFuture<Result<(), RegisterError>> {
        let nacos = self.clone();
        Box::pin(async move {
            let group = nacos.group.clone();
            let service_name = get_service_name(resource.as_ref(), &protocol);
            let instance = build_instance(resource.as_ref());
            info!("nacos register service: {service_name} - group: {group:?}");
            let ret = nacos
                .naming_service
                .register_instance(service_name, group, instance)
                .await;
            if let Err(error) = ret {
                error!("nacos register to nacos occur an error: {error:?}");
                return Err(RegisterError::Error(Box::new(error)));
            }
            Ok(())
        })
    }

    fn deregister(
        &self,
        resource: Arc<ServiceResource>,
        protocol: Protocol,
    ) -> BoxFuture<Result<(), RegisterError>> {
        let nacos = self.clone();
        Box::pin(async move {
            let group = nacos.group.clone();
            let service_name = get_service_name(resource.as_ref(), &protocol);
            let instance = build_instance(resource.as_ref());
            info!("nacos deregister service: {service_name} - group: {group:?}");
            let ret = nacos
                .naming_service
                .deregister_instance(service_name, group, instance)
                .await;
            if let Err(error) = ret {
                error!("nacos deregister to nacos occur an error: {error:?}",);
                return Err(RegisterError::Error(Box::new(error)));
            }
            Ok(())
        })
    }

    fn subscribe(
        &self,
        resource: ServiceResource,
        protocol: Protocol,
    ) -> BoxFuture<Result<Directory, RegisterError>> {
        let nacos = self.clone();
        Box::pin(async move {
            let group = nacos.group.clone();
            let service_name = get_service_name(&resource, &protocol);
            info!("subscribe service: {service_name} - grep: {group:?}");
            let directory = Directory::default();
            let directory_clone = directory.clone();
            let naming_service = nacos.naming_service.clone();
            let service_instances = naming_service
                .get_all_instances(service_name.clone(), group.clone(), Vec::new(), false)
                .await
                .map_err(|error| RegisterError::Error(Box::new(error)))?;
            let service_instances = to_service_resources(service_instances);
            directory.change(service_instances).await?;
            let event_listener = ServiceChangeListener::new(directory);
            let event_listener = Arc::new(event_listener);
            naming_service
                .subscribe(service_name, group, Vec::new(), event_listener)
                .await
                .map_err(|error| RegisterError::Error(Box::new(error)))?;
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
        info!("service change: {}", event.service_name);
        info!("nacos event: {event:?}");
        let directory = self.directory.clone();
        let instances = event.instances.to_owned();
        tokio::spawn(async move {
            let instances = instances;
            let resources = if let Some(instances) = instances {
                to_service_resources(instances)
            } else {
                vec![]
            };
            let _ = directory.change(resources).await;
        });
    }
}

fn to_service_resources(service_instances: Vec<ServiceInstance>) -> Vec<ServiceResource> {
    service_instances.into_iter().fold(vec![], |mut vec, e| {
        let resource = ServiceResource {
            addr: format!("http://{}:{}", e.ip(), e.port),
            service_id: e.service_name.unwrap_or_default(),
            group: None,
            version: None,
            methods: Default::default(),
            weight: Some(e.weight),
            metadata: e.metadata,
        };
        vec.push(resource);
        vec
    })
}

pub fn get_service_name(resource: &ServiceResource, protocol: &Protocol) -> String {
    match &protocol {
        &Protocol::SpringCloud(app_name) => app_name.clone(),
        &Protocol::Dubbo | Protocol::Fusen => format!(
            "providers:{}:{}:{}",
            resource.service_id,
            resource.version.as_ref().map_or("", |e| e),
            resource.group.as_ref().map_or("", |e| e),
        ),
        _ => unimplemented!(),
    }
}

fn build_instance(resource: &ServiceResource) -> ServiceInstance {
    let (ip, port) = resource.addr.split_once(':').unwrap();
    nacos_sdk::api::naming::ServiceInstance {
        ip: ip.to_string(),
        port: port.parse().unwrap(),
        metadata: resource.metadata.clone(),
        ..Default::default()
    }
}
