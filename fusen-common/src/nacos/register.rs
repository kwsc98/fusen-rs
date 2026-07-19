use crate::{error::Error, nacos::NacosConfig};
use fusen_register::{
    Register,
    directory::Directory,
    error::RegisterError,
    fusen_internal_common::{
        BoxFuture, protocol::WireProtocol, resource::service::ServiceResource,
    },
};
use nacos_sdk::api::{
    naming::{
        NamingChangeEvent, NamingEventListener, NamingService, NamingServiceBuilder,
        ServiceInstance,
    },
    props::ClientProps,
};
use std::sync::Arc;

#[derive(Clone)]
pub struct NacosRegister {
    naming_service: Arc<NamingService>,
}

impl NacosRegister {
    pub fn init_nacos_register(app_name: &str, config: Arc<NacosConfig>) -> Result<Self, Error> {
        let props = ClientProps::new()
            .server_addr(config.server_addr.clone())
            .namespace(config.namespace.clone().unwrap_or_default())
            .app_name(app_name)
            .auth_username(config.username.clone().unwrap_or_default())
            .auth_password(config.password.clone().unwrap_or_default());
        let builder = NamingServiceBuilder::new(props);
        let builder = if config.username.is_some() {
            builder.enable_auth_plugin_http()
        } else {
            builder
        };
        Ok(Self {
            naming_service: Arc::new(builder.build().map_err(Error::register)?),
        })
    }
}

impl Register for NacosRegister {
    fn register(
        &self,
        resource: Arc<ServiceResource>,
        protocol: WireProtocol,
    ) -> BoxFuture<Result<(), RegisterError>> {
        let naming = self.naming_service.clone();
        Box::pin(async move {
            let service_name = get_service_name(&resource, protocol);
            let instance = build_instance(&resource)?;
            naming
                .register_instance(service_name, resource.group.clone(), instance)
                .await
                .map_err(|error| RegisterError::Error(Box::new(error)))
        })
    }

    fn deregister(
        &self,
        resource: Arc<ServiceResource>,
        protocol: WireProtocol,
    ) -> BoxFuture<Result<(), RegisterError>> {
        let naming = self.naming_service.clone();
        Box::pin(async move {
            let service_name = get_service_name(&resource, protocol);
            let instance = build_instance(&resource)?;
            naming
                .deregister_instance(service_name, resource.group.clone(), instance)
                .await
                .map_err(|error| RegisterError::Error(Box::new(error)))
        })
    }

    fn subscribe(
        &self,
        resource: ServiceResource,
        protocol: WireProtocol,
    ) -> BoxFuture<Result<Directory, RegisterError>> {
        let naming = self.naming_service.clone();
        Box::pin(async move {
            let service_name = get_service_name(&resource, protocol);
            let instances = naming
                .get_all_instances(
                    service_name.clone(),
                    resource.group.clone(),
                    Vec::new(),
                    false,
                )
                .await
                .map_err(|error| RegisterError::Error(Box::new(error)))?;
            let directory = Directory::default();
            directory.replace(to_service_resources(instances))?;
            naming
                .subscribe(
                    service_name,
                    resource.group,
                    Vec::new(),
                    Arc::new(ServiceChangeListener {
                        directory: directory.clone(),
                    }),
                )
                .await
                .map_err(|error| RegisterError::Error(Box::new(error)))?;
            Ok(directory)
        })
    }
}

#[derive(Clone)]
struct ServiceChangeListener {
    directory: Directory,
}

impl NamingEventListener for ServiceChangeListener {
    fn event(&self, event: Arc<NamingChangeEvent>) {
        let directory = self.directory.clone();
        let resources = event
            .instances
            .clone()
            .map(to_service_resources)
            .unwrap_or_default();
        if let Err(error) = directory.replace(resources) {
            tracing::error!(?error, service = %event.service_name, "failed to update service directory");
        }
    }
}

fn to_service_resources(instances: Vec<ServiceInstance>) -> Vec<ServiceResource> {
    instances
        .into_iter()
        .map(|instance| ServiceResource {
            addr: format!("http://{}:{}", instance.ip(), instance.port),
            service_id: instance.service_name.unwrap_or_default(),
            group: None,
            version: None,
            methods: Vec::new(),
            weight: Some(instance.weight),
            metadata: instance.metadata,
        })
        .collect()
}

pub fn get_service_name(resource: &ServiceResource, protocol: WireProtocol) -> String {
    match protocol {
        WireProtocol::SpringCloud => resource
            .metadata
            .get("spring.application.name")
            .cloned()
            .unwrap_or_else(|| resource.service_id.clone()),
        WireProtocol::Fusen => format!(
            "providers:{}:{}:{}",
            resource.service_id,
            resource.version.as_deref().unwrap_or(""),
            resource.group.as_deref().unwrap_or("")
        ),
    }
}

fn build_instance(resource: &ServiceResource) -> Result<ServiceInstance, RegisterError> {
    let url = url::Url::parse(&resource.addr)
        .map_err(|error| RegisterError::InvalidResource(error.to_string()))?;
    let ip = url
        .host_str()
        .ok_or_else(|| RegisterError::InvalidResource("advertised URL has no host".into()))?;
    let port = url
        .port_or_known_default()
        .ok_or_else(|| RegisterError::InvalidResource("advertised URL has no port".into()))?;
    Ok(ServiceInstance {
        ip: ip.to_owned(),
        port: i32::from(port),
        metadata: resource.metadata.clone(),
        ..Default::default()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn resource(addr: &str) -> ServiceResource {
        ServiceResource {
            service_id: "demo".into(),
            group: Some("DEFAULT_GROUP".into()),
            version: Some("1.0".into()),
            methods: Vec::new(),
            addr: addr.into(),
            weight: Some(1.0),
            metadata: Default::default(),
        }
    }

    #[test]
    fn rejects_invalid_advertised_url() {
        assert!(matches!(
            build_instance(&resource("127.0.0.1:8080")),
            Err(RegisterError::InvalidResource(_))
        ));
    }

    #[test]
    fn spring_cloud_service_name_uses_metadata() {
        let mut resource = resource("http://127.0.0.1:8080");
        resource
            .metadata
            .insert("spring.application.name".into(), "orders".into());
        assert_eq!(
            get_service_name(&resource, WireProtocol::SpringCloud),
            "orders"
        );
    }

    #[tokio::test]
    async fn live_nacos_registration_when_configured() {
        let Ok(server_addr) = std::env::var("NACOS_ADDR") else {
            return;
        };
        let register = NacosRegister::init_nacos_register(
            "fusen-test",
            Arc::new(NacosConfig {
                server_addr,
                ..Default::default()
            }),
        )
        .unwrap();
        let resource = Arc::new(resource("http://127.0.0.1:18081"));
        register
            .register(resource.clone(), WireProtocol::Fusen)
            .await
            .unwrap();
        register
            .deregister(resource, WireProtocol::Fusen)
            .await
            .unwrap();
    }
}
