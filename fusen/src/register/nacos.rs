use std::sync::Arc;

use fusen_macro::UrlConfig;
use nacos_sdk::api::{
    naming::{NamingService, NamingServiceBuilder},
    props::ClientProps,
};

use super::{Directory, RegisterBuilder, RegisterV2};

pub struct FusenNacos {
    naming_service: Box<dyn NamingService + Sync + Send + 'static>,
}

#[derive(UrlConfig)]
struct NacosConfig {
    server_addr: String,
    namespace: String,
    app_name: String,
    username: String,
    password: String,
}

impl FusenNacos {
    async fn init(url: &str) -> Self {
        let mut client_props = ClientProps::new();
        client_props = client_props
            .server_addr(server_addr)
            .namespace(namespace)
            .app_name(app_name)
            .auth_username(username)
            .auth_password(password);
        let mut naming_service = NamingServiceBuilder::new(client_props).build().unwrap();
        Self {
            naming_service: Box::new(naming_service),
        }
    }
}

impl RegisterV2 for FusenNacos {
    async fn register(&self, resource: super::Resource) -> Result<(), crate::Error> {
        todo!()
    }

    async fn subscribe(&self, resource: super::Resource) -> Result<Directory, crate::Error> {
        todo!()
    }
}
