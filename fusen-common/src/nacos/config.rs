use crate::{
    config::{ConfigManager, ConfigResponse, HotConfigChangeListener, config_build},
    error::Error,
    nacos::NacosConfig,
};
use nacos_sdk::api::{
    config::{ConfigChangeListener, ConfigService, ConfigServiceBuilder},
    props::ClientProps,
};
use std::sync::Arc;

#[derive(Clone)]
pub struct NacosConfiguration {
    config_service: Arc<ConfigService>,
}

impl NacosConfiguration {
    pub async fn init_nacos_configuration(config: Arc<NacosConfig>) -> Result<Self, Error> {
        let props = ClientProps::new()
            .server_addr(config.server_addr.clone())
            .namespace(config.namespace.clone().unwrap_or_default())
            .auth_username(config.username.clone().unwrap_or_default())
            .auth_password(config.password.clone().unwrap_or_default());
        let builder = ConfigServiceBuilder::new(props);
        let builder = if config.username.is_some() {
            builder.enable_auth_plugin_http()
        } else {
            builder
        };
        Ok(Self {
            config_service: Arc::new(builder.build().map_err(Error::config)?),
        })
    }

    pub async fn get_config<T: serde::de::DeserializeOwned>(
        &self,
        data_id: &str,
        group: &str,
    ) -> Result<T, Error> {
        let response = self
            .config_service
            .get_config(data_id.to_owned(), group.to_owned())
            .await
            .map_err(Error::config)?;
        config_build(ConfigResponse {
            content_type: response.content_type().to_owned(),
            content: response.content().to_owned(),
        })
    }

    pub async fn get_config_manager<T: serde::de::DeserializeOwned + Send + Sync + 'static>(
        &self,
        data_id: &str,
        group: &str,
    ) -> Result<ConfigManager<T>, Error> {
        let initial = self.get_config(data_id, group).await?;
        let (listener, receiver) = HotConfigChangeListener::new();
        let manager = ConfigManager::build_hot_config(initial, receiver)?;
        self.config_service
            .add_listener(data_id.to_owned(), group.to_owned(), Arc::new(listener))
            .await
            .map_err(Error::config)?;
        Ok(manager)
    }
}

impl ConfigChangeListener for HotConfigChangeListener {
    fn notify(&self, response: nacos_sdk::api::config::ConfigResponse) {
        let sender = self.sender.clone();
        tokio::spawn(async move {
            if let Err(error) = sender
                .send(ConfigResponse {
                    content_type: response.content_type().to_owned(),
                    content: response.content().to_owned(),
                })
                .await
            {
                tracing::error!(?error, "Nacos configuration listener is closed");
            }
        });
    }
}
