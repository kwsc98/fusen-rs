use crate::error::Error;
use std::{fs, future::Future, pin::Pin, sync::Arc};
use tokio::sync::watch;

#[doc(hidden)]
pub type ConfigCloseFuture = Pin<Box<dyn Future<Output = Result<(), Error>> + Send + 'static>>;

#[doc(hidden)]
pub trait ConfigLifecycle: Send + Sync {
    fn request_close(&self);
    fn close(&self) -> ConfigCloseFuture;
}

/// Provider-facing sender used to publish raw hot-configuration updates.
pub struct HotConfigChangeListener {
    /// Latest-wins update channel.
    pub sender: watch::Sender<Option<ConfigResponse>>,
}

#[derive(Clone)]
/// Raw configuration content and its declared serialization format.
pub struct ConfigResponse {
    /// Serialization format such as `toml`, `yaml`, or `yml`.
    pub content_type: String,
    /// Raw configuration text.
    pub content: String,
}

impl ConfigResponse {
    /// Returns the declared serialization format.
    pub fn content_type(&self) -> &str {
        &self.content_type
    }

    /// Returns the raw configuration text.
    pub fn content(&self) -> &str {
        &self.content
    }
}

impl HotConfigChangeListener {
    /// Creates a publisher and its latest-wins receiver.
    pub fn new() -> (Self, watch::Receiver<Option<ConfigResponse>>) {
        let (sender, receiver) = watch::channel(None);
        (Self { sender }, receiver)
    }
}

/// Read-only access to one atomically replaced typed configuration value.
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
    /// Starts a typed hot-configuration manager from an initial value and raw update stream.
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

    #[doc(hidden)]
    pub fn with_lifecycle(mut self, lifecycle: Arc<dyn ConfigLifecycle>) -> Self {
        self.lifecycle = Some(lifecycle);
        self
    }

    /// Returns the latest successfully parsed configuration snapshot.
    pub fn get_hot_config(&self) -> Arc<T> {
        self.receiver.borrow().clone()
    }

    /// Waits for and returns the next successfully parsed configuration snapshot.
    pub async fn changed(&mut self) -> Result<Arc<T>, Error> {
        self.receiver
            .changed()
            .await
            .map_err(|_| Error::Message("hot configuration channel closed".into()))?;
        Ok(self.receiver.borrow().clone())
    }

    /// Explicitly closes the attached provider listener, when one exists.
    pub async fn close(&self) -> Result<(), Error> {
        match &self.lifecycle {
            Some(lifecycle) => lifecycle.close().await,
            None => Ok(()),
        }
    }
}

/// Deserializes one raw configuration response according to its content type.
pub fn config_build<T: serde::de::DeserializeOwned>(response: ConfigResponse) -> Result<T, Error> {
    match response.content_type().to_ascii_lowercase().as_str() {
        "toml" => get_toml_by_context(response.content()),
        "yaml" | "yml" => get_yaml_by_context(response.content()),
        value => Err(Error::Message(format!(
            "unsupported configuration type {value}"
        ))),
    }
}

/// Deserializes TOML text into a typed configuration value.
pub fn get_toml_by_context<T: serde::de::DeserializeOwned>(content: &str) -> Result<T, Error> {
    toml::from_str(content).map_err(Error::config)
}

/// Deserializes YAML text when the `yaml` feature is enabled.
pub fn get_yaml_by_context<T: serde::de::DeserializeOwned>(content: &str) -> Result<T, Error> {
    #[cfg(feature = "yaml")]
    {
        serde_yaml_ng::from_str(content).map_err(Error::config)
    }
    #[cfg(not(feature = "yaml"))]
    {
        let _ = content;
        Err(Error::Message(
            "YAML support requires the fusen-config `yaml` feature".into(),
        ))
    }
}

/// Reads and deserializes a TOML or YAML configuration file based on its extension.
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
    use std::sync::atomic::{AtomicUsize, Ordering};

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

    #[derive(Default)]
    struct RecordingLifecycle {
        close_requests: AtomicUsize,
        close_waits: AtomicUsize,
    }

    impl ConfigLifecycle for RecordingLifecycle {
        fn request_close(&self) {
            self.close_requests.fetch_add(1, Ordering::SeqCst);
        }

        fn close(&self) -> ConfigCloseFuture {
            self.close_waits.fetch_add(1, Ordering::SeqCst);
            Box::pin(async { Ok(()) })
        }
    }

    #[tokio::test]
    async fn hot_config_updates_and_explicit_close_use_the_attached_lifecycle() {
        let (listener, receiver) = HotConfigChangeListener::new();
        let lifecycle = Arc::new(RecordingLifecycle::default());
        let mut manager = ConfigManager::build_hot_config(
            Demo {
                value: "initial".into(),
            },
            receiver,
        )
        .unwrap()
        .with_lifecycle(lifecycle.clone());

        listener.sender.send_replace(Some(ConfigResponse {
            content_type: "toml".into(),
            content: "value = 'updated'".into(),
        }));
        assert_eq!(manager.changed().await.unwrap().value, "updated");
        manager.close().await.unwrap();
        assert_eq!(lifecycle.close_waits.load(Ordering::SeqCst), 1);

        drop(manager);
        assert_eq!(lifecycle.close_requests.load(Ordering::SeqCst), 1);
    }
}
