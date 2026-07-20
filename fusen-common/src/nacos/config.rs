use crate::{
    config::{
        ConfigCloseFuture, ConfigLifecycle, ConfigManager, ConfigResponse, HotConfigChangeListener,
        config_build,
    },
    error::Error,
    nacos::NacosConfig,
};
use nacos_sdk::api::{
    config::{ConfigChangeListener, ConfigService, ConfigServiceBuilder},
    props::ClientProps,
};
use std::sync::Arc;
use tokio::sync::watch;

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
        config_build(self.get_config_response(data_id, group).await?)
    }

    async fn get_config_response(
        &self,
        data_id: &str,
        group: &str,
    ) -> Result<ConfigResponse, Error> {
        let response = self
            .config_service
            .get_config(data_id.to_owned(), group.to_owned())
            .await
            .map_err(Error::config)?;
        Ok(ConfigResponse {
            content_type: response.content_type().to_owned(),
            content: response.content().to_owned(),
        })
    }

    pub async fn get_config_manager<T: serde::de::DeserializeOwned + Send + Sync + 'static>(
        &self,
        data_id: &str,
        group: &str,
    ) -> Result<ConfigManager<T>, Error> {
        let (listener, mut receiver) = HotConfigChangeListener::new();
        let listener: Arc<dyn ConfigChangeListener> = Arc::new(listener);
        self.config_service
            .add_listener(data_id.to_owned(), group.to_owned(), listener.clone())
            .await
            .map_err(Error::config)?;
        let lifecycle = Arc::new(NacosConfigLifecycle::new(
            self.config_service.clone(),
            data_id.to_owned(),
            group.to_owned(),
            listener,
        ));
        let setup_guard = ConfigSetupGuard::new(lifecycle);
        let fetched = self.get_config_response(data_id, group).await?;
        let initial = config_build(select_initial_response(fetched, &mut receiver))?;
        let manager = ConfigManager::build_hot_config(initial, receiver)?;
        Ok(manager.with_lifecycle(setup_guard.disarm()))
    }
}

fn select_initial_response(
    fetched: ConfigResponse,
    receiver: &mut watch::Receiver<Option<ConfigResponse>>,
) -> ConfigResponse {
    receiver.borrow_and_update().clone().unwrap_or(fetched)
}

impl ConfigChangeListener for HotConfigChangeListener {
    fn notify(&self, response: nacos_sdk::api::config::ConfigResponse) {
        self.sender.send_replace(Some(ConfigResponse {
            content_type: response.content_type().to_owned(),
            content: response.content().to_owned(),
        }));
    }
}

struct NacosConfigLifecycle {
    cancel: watch::Sender<bool>,
    result: watch::Receiver<Option<Result<(), Error>>>,
}

impl NacosConfigLifecycle {
    fn new(
        service: Arc<ConfigService>,
        data_id: String,
        group: String,
        listener: Arc<dyn ConfigChangeListener>,
    ) -> Self {
        let (cancel, mut receiver) = watch::channel(false);
        let (result_sender, result) = watch::channel(None);
        tokio::spawn(async move {
            if !*receiver.borrow() {
                let _ = receiver.changed().await;
            }
            let result = service
                .remove_listener(data_id, group, listener)
                .await
                .map_err(Error::config);
            result_sender.send_replace(Some(result));
        });
        Self { cancel, result }
    }
}

impl ConfigLifecycle for NacosConfigLifecycle {
    fn request_close(&self) {
        self.cancel.send_replace(true);
    }

    fn close(&self) -> ConfigCloseFuture {
        self.request_close();
        let mut result = self.result.clone();
        Box::pin(async move {
            loop {
                if let Some(result) = result.borrow().clone() {
                    return result;
                }
                result.changed().await.map_err(|_| {
                    Error::config(std::io::Error::other(
                        "Nacos config listener cleanup ended without a result",
                    ))
                })?;
            }
        })
    }
}

struct ConfigSetupGuard {
    lifecycle: Option<Arc<NacosConfigLifecycle>>,
}

impl ConfigSetupGuard {
    fn new(lifecycle: Arc<NacosConfigLifecycle>) -> Self {
        Self {
            lifecycle: Some(lifecycle),
        }
    }

    fn disarm(mut self) -> Arc<NacosConfigLifecycle> {
        self.lifecycle
            .take()
            .expect("setup lifecycle is present until disarmed")
    }
}

impl Drop for ConfigSetupGuard {
    fn drop(&mut self) {
        if let Some(lifecycle) = &self.lifecycle {
            lifecycle.request_close();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn response(value: &str) -> ConfigResponse {
        ConfigResponse {
            content_type: "toml".into(),
            content: format!("value = '{value}'"),
        }
    }

    #[test]
    fn listener_update_wins_over_older_initial_fetch() {
        let (listener, mut receiver) = HotConfigChangeListener::new();
        listener.sender.send_replace(Some(response("latest")));
        let selected = select_initial_response(response("stale"), &mut receiver);
        assert_eq!(selected.content, "value = 'latest'");
    }

    #[test]
    fn latest_initialization_update_wins() {
        let (listener, mut receiver) = HotConfigChangeListener::new();
        listener.sender.send_replace(Some(response("old")));
        listener.sender.send_replace(Some(response("latest")));
        let selected = select_initial_response(response("stale"), &mut receiver);
        assert_eq!(selected.content, "value = 'latest'");
    }
}
