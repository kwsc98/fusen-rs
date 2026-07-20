use crate::error::Error;
use std::{fs, future::Future, pin::Pin, sync::Arc};
use tokio::sync::watch;

pub(crate) type ConfigCloseFuture =
    Pin<Box<dyn Future<Output = Result<(), Error>> + Send + 'static>>;

pub(crate) trait ConfigLifecycle: Send + Sync {
    fn request_close(&self);
    fn close(&self) -> ConfigCloseFuture;
}

pub struct HotConfigChangeListener {
    pub sender: watch::Sender<Option<ConfigResponse>>,
}

#[derive(Clone)]
pub struct ConfigResponse {
    pub content_type: String,
    pub content: String,
}

impl ConfigResponse {
    pub fn content_type(&self) -> &str {
        &self.content_type
    }

    pub fn content(&self) -> &str {
        &self.content
    }
}

impl HotConfigChangeListener {
    pub fn new() -> (Self, watch::Receiver<Option<ConfigResponse>>) {
        let (sender, receiver) = watch::channel(None);
        (Self { sender }, receiver)
    }
}

pub struct ConfigManager<T> {
    receiver: watch::Receiver<Arc<T>>,
    lifecycle: Option<Arc<dyn ConfigLifecycle>>,
}

impl<T> Drop for ConfigManager<T> {
    fn drop(&mut self) {
        if let Some(lifecycle) = &self.lifecycle {
            lifecycle.request_close();
        }
    }
}

impl<T> ConfigManager<T>
where
    T: serde::de::DeserializeOwned + Send + Sync + 'static,
{
    pub fn build_hot_config(
        config: T,
        mut listener: watch::Receiver<Option<ConfigResponse>>,
    ) -> Result<Self, Error> {
        let (sender, receiver) = watch::channel(Arc::new(config));
        tokio::spawn(async move {
            while listener.changed().await.is_ok() {
                let Some(response) = listener.borrow_and_update().clone() else {
                    continue;
                };
                match config_build(response) {
                    Ok(config) => {
                        if sender.send(Arc::new(config)).is_err() {
                            break;
                        }
                    }
                    Err(error) => tracing::error!(?error, "hot configuration update rejected"),
                }
            }
        });
        Ok(Self {
            receiver,
            lifecycle: None,
        })
    }

    pub(crate) fn with_lifecycle(mut self, lifecycle: Arc<dyn ConfigLifecycle>) -> Self {
        self.lifecycle = Some(lifecycle);
        self
    }

    pub fn get_hot_config(&self) -> Arc<T> {
        self.receiver.borrow().clone()
    }

    pub async fn changed(&mut self) -> Result<Arc<T>, Error> {
        self.receiver
            .changed()
            .await
            .map_err(|_| Error::Message("hot configuration channel closed".into()))?;
        Ok(self.receiver.borrow().clone())
    }

    pub async fn close(&self) -> Result<(), Error> {
        match &self.lifecycle {
            Some(lifecycle) => lifecycle.close().await,
            None => Ok(()),
        }
    }
}

pub fn config_build<T: serde::de::DeserializeOwned>(response: ConfigResponse) -> Result<T, Error> {
    match response.content_type().to_ascii_lowercase().as_str() {
        "toml" => get_toml_by_context(response.content()),
        "yaml" | "yml" => get_yaml_by_context(response.content()),
        value => Err(Error::Message(format!(
            "unsupported configuration type {value}"
        ))),
    }
}

pub fn get_toml_by_context<T: serde::de::DeserializeOwned>(content: &str) -> Result<T, Error> {
    toml::from_str(content).map_err(Error::config)
}

pub fn get_yaml_by_context<T: serde::de::DeserializeOwned>(content: &str) -> Result<T, Error> {
    #[cfg(feature = "yaml")]
    {
        serde_yaml_ng::from_str(content).map_err(Error::config)
    }
    #[cfg(not(feature = "yaml"))]
    {
        let _ = content;
        Err(Error::Message(
            "YAML support requires the fusen-common `yaml` feature".into(),
        ))
    }
}

pub fn get_config_by_path<T: serde::de::DeserializeOwned>(path: &str) -> Result<T, Error> {
    let content = fs::read_to_string(path).map_err(Error::config)?;
    match std::path::Path::new(path)
        .extension()
        .and_then(|value| value.to_str())
    {
        Some("toml") => get_toml_by_context(&content),
        Some("yaml" | "yml") => get_yaml_by_context(&content),
        extension => Err(Error::Message(format!(
            "unsupported configuration extension {extension:?}"
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::Deserialize;

    #[derive(Deserialize)]
    struct Demo {
        value: String,
    }

    #[test]
    fn parses_toml() {
        let config: Demo = get_toml_by_context("value = 'ok'").unwrap();
        assert_eq!(config.value, "ok");
    }

    #[tokio::test]
    async fn hot_updates_keep_only_the_latest_value() {
        let (listener, receiver) = HotConfigChangeListener::new();
        let mut manager = ConfigManager::build_hot_config(
            Demo {
                value: "initial".into(),
            },
            receiver,
        )
        .unwrap();
        listener.sender.send_replace(Some(ConfigResponse {
            content_type: "toml".into(),
            content: "value = 'old'".into(),
        }));
        listener.sender.send_replace(Some(ConfigResponse {
            content_type: "toml".into(),
            content: "value = 'latest'".into(),
        }));
        assert_eq!(manager.changed().await.unwrap().value, "latest");
    }
}
