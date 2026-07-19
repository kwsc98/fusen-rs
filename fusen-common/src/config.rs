use crate::error::Error;
use std::{fs, sync::Arc};
use tokio::sync::{mpsc, watch};

pub struct HotConfigChangeListener {
    pub sender: mpsc::Sender<ConfigResponse>,
}

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
    pub fn new() -> (Self, mpsc::Receiver<ConfigResponse>) {
        let (sender, receiver) = mpsc::channel(16);
        (Self { sender }, receiver)
    }
}

pub struct ConfigManager<T> {
    receiver: watch::Receiver<Arc<T>>,
}

impl<T> ConfigManager<T>
where
    T: serde::de::DeserializeOwned + Send + Sync + 'static,
{
    pub fn build_hot_config(
        config: T,
        mut listener: mpsc::Receiver<ConfigResponse>,
    ) -> Result<Self, Error> {
        let (sender, receiver) = watch::channel(Arc::new(config));
        tokio::spawn(async move {
            while let Some(response) = listener.recv().await {
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
        Ok(Self { receiver })
    }

    pub async fn get_hot_config(&self) -> Arc<T> {
        self.receiver.borrow().clone()
    }

    pub async fn changed(&mut self) -> Result<Arc<T>, Error> {
        self.receiver
            .changed()
            .await
            .map_err(|_| Error::Message("hot configuration channel closed".into()))?;
        Ok(self.receiver.borrow().clone())
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
    serde_yaml::from_str(content).map_err(Error::config)
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
}
