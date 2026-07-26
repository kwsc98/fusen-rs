use crate::{NacosConfig, client_props, validate_application_name};
use fusen_config::{
    ConfigDocument, ConfigError, ConfigErrorKind, ConfigFormat, ConfigFuture, ConfigHandle,
    ConfigKey, ConfigOperation, ConfigPublisher, ConfigSource, prepare_config,
};
use nacos_sdk::api::config::{
    ConfigChangeListener, ConfigResponse, ConfigService, ConfigServiceBuilder,
};
use nacos_sdk::api::error::Error as NacosError;
use std::sync::Arc;

const DEFAULT_GROUP: &str = "DEFAULT_GROUP";

/// Nacos-backed hot configuration source.
#[derive(Clone)]
pub struct NacosConfigSource {
    client: Arc<dyn ConfigOperations>,
}

impl NacosConfigSource {
    /// Connects a Nacos configuration client for one application.
    ///
    /// Configuration is validated before the SDK performs network I/O.
    pub async fn connect(
        application_name: impl Into<String>,
        config: NacosConfig,
    ) -> Result<Self, ConfigError> {
        let application_name = application_name.into();
        config.validate().map_err(|message| {
            ConfigError::message(
                ConfigOperation::Prepare,
                ConfigErrorKind::InvalidInput,
                message,
            )
        })?;
        validate_application_name(&application_name).map_err(|message| {
            ConfigError::message(
                ConfigOperation::Prepare,
                ConfigErrorKind::InvalidInput,
                message,
            )
        })?;
        let builder = ConfigServiceBuilder::new(client_props(&config, &application_name));
        let builder = if config.username().is_some() {
            builder.enable_auth_plugin_http()
        } else {
            builder
        };
        let service = builder
            .build()
            .await
            .map_err(|error| provider_error(ConfigOperation::Prepare, error))?;
        Ok(Self {
            client: Arc::new(SdkConfigOperations {
                service: Arc::new(service),
            }),
        })
    }
}

impl std::fmt::Debug for NacosConfigSource {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("NacosConfigSource")
            .finish_non_exhaustive()
    }
}

impl ConfigSource for NacosConfigSource {
    fn prepare(&self, key: ConfigKey) -> Result<ConfigHandle, ConfigError> {
        let data_id = key.name().to_owned();
        let group = key.group().unwrap_or(DEFAULT_GROUP).to_owned();
        let activate_client = self.client.clone();
        let close_client = self.client.clone();

        Ok(prepare_config(move |publisher| {
            let listener: Arc<dyn ConfigChangeListener> = Arc::new(NacosConfigChangeListener {
                data_id: data_id.clone(),
                publisher,
            });
            let activate_data_id = data_id.clone();
            let activate_group = group.clone();
            let activate_listener = listener.clone();
            (
                async move {
                    activate_client
                        .add_listener(
                            activate_data_id.clone(),
                            activate_group.clone(),
                            activate_listener,
                        )
                        .await?;
                    activate_client.get(activate_data_id, activate_group).await
                },
                move || async move { close_client.remove_listener(data_id, group, listener).await },
            )
        }))
    }
}

trait ConfigOperations: Send + Sync {
    fn add_listener(
        &self,
        data_id: String,
        group: String,
        listener: Arc<dyn ConfigChangeListener>,
    ) -> ConfigFuture<()>;

    fn get(&self, data_id: String, group: String) -> ConfigFuture<ConfigDocument>;

    fn remove_listener(
        &self,
        data_id: String,
        group: String,
        listener: Arc<dyn ConfigChangeListener>,
    ) -> ConfigFuture<()>;
}

struct SdkConfigOperations {
    service: Arc<ConfigService>,
}

impl ConfigOperations for SdkConfigOperations {
    fn add_listener(
        &self,
        data_id: String,
        group: String,
        listener: Arc<dyn ConfigChangeListener>,
    ) -> ConfigFuture<()> {
        let service = self.service.clone();
        Box::pin(async move {
            service
                .add_listener(data_id, group, listener)
                .await
                .map_err(|error| provider_error(ConfigOperation::Activate, error))
        })
    }

    fn get(&self, data_id: String, group: String) -> ConfigFuture<ConfigDocument> {
        let service = self.service.clone();
        Box::pin(async move {
            let response = service
                .get_config(data_id.clone(), group)
                .await
                .map_err(|error| provider_error(ConfigOperation::Activate, error))?;
            document_from_response(response, &data_id, ConfigOperation::Activate)
        })
    }

    fn remove_listener(
        &self,
        data_id: String,
        group: String,
        listener: Arc<dyn ConfigChangeListener>,
    ) -> ConfigFuture<()> {
        let service = self.service.clone();
        Box::pin(async move {
            service
                .remove_listener(data_id, group, listener)
                .await
                .map_err(|error| provider_error(ConfigOperation::Close, error))
        })
    }
}

fn provider_error(operation: ConfigOperation, error: NacosError) -> ConfigError {
    let kind = match &error {
        NacosError::InvalidParam(_, _) | NacosError::WrongServerAddress(_) => {
            ConfigErrorKind::InvalidInput
        }
        NacosError::Serialization(_) => ConfigErrorKind::InvalidData,
        _ => ConfigErrorKind::Unavailable,
    };
    ConfigError::new(operation, kind, error)
}

struct NacosConfigChangeListener {
    data_id: String,
    publisher: ConfigPublisher,
}

impl ConfigChangeListener for NacosConfigChangeListener {
    fn notify(&self, response: ConfigResponse) {
        let document = document_from_response(response, &self.data_id, ConfigOperation::Publish);
        match document.and_then(|document| self.publisher.publish(document)) {
            Ok(()) => {}
            Err(error) => tracing::warn!(
                error = %error,
                data_id = %self.data_id,
                "Nacos configuration update rejected"
            ),
        }
    }
}

fn document_from_response(
    response: ConfigResponse,
    data_id: &str,
    operation: ConfigOperation,
) -> Result<ConfigDocument, ConfigError> {
    let format = ConfigFormat::from_name(response.content_type())
        .or_else(|| {
            std::path::Path::new(data_id)
                .extension()
                .and_then(|extension| extension.to_str())
                .and_then(ConfigFormat::from_name)
        })
        .ok_or_else(|| {
            ConfigError::message(
                operation,
                ConfigErrorKind::UnsupportedFormat,
                format!(
                    "Nacos resource {data_id:?} uses unsupported content type {:?}",
                    response.content_type()
                ),
            )
        })?;
    Ok(ConfigDocument::new(format, response.content().to_owned()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::Deserialize;
    use std::sync::{
        Mutex,
        atomic::{AtomicUsize, Ordering},
    };
    use tokio::sync::{Notify, oneshot};

    #[derive(Debug, Deserialize)]
    struct Demo {
        value: String,
    }

    fn response(data_id: &str, content_type: &str, content: &str) -> ConfigResponse {
        ConfigResponse::new(
            data_id.into(),
            DEFAULT_GROUP.into(),
            String::new(),
            content.into(),
            content_type.into(),
            String::new(),
        )
    }

    #[test]
    fn format_uses_content_type_then_data_id_extension() {
        let explicit = document_from_response(
            response("settings.data", "toml", "value = 'ok'"),
            "settings.data",
            ConfigOperation::Activate,
        )
        .unwrap();
        assert_eq!(explicit.format(), ConfigFormat::Toml);

        let inferred = document_from_response(
            response("settings.yaml", "text", "value: ok"),
            "settings.yaml",
            ConfigOperation::Activate,
        )
        .unwrap();
        assert_eq!(inferred.format(), ConfigFormat::Yaml);
    }

    #[test]
    fn unknown_format_is_rejected_without_exposing_content() {
        let error = document_from_response(
            response("settings.data", "xml", "<password>secret</password>"),
            "settings.data",
            ConfigOperation::Activate,
        )
        .unwrap_err();
        assert_eq!(error.kind(), ConfigErrorKind::UnsupportedFormat);
        assert!(!error.to_string().contains("secret"));
    }

    #[derive(Clone)]
    struct ControlledConfig {
        listener_ready: Arc<Notify>,
        listener: Arc<Mutex<Option<Arc<dyn ConfigChangeListener>>>>,
        fetch_release: Arc<Mutex<Option<oneshot::Receiver<()>>>>,
        removals: Arc<AtomicUsize>,
    }

    impl ConfigOperations for ControlledConfig {
        fn add_listener(
            &self,
            _data_id: String,
            _group: String,
            listener: Arc<dyn ConfigChangeListener>,
        ) -> ConfigFuture<()> {
            let provider = self.clone();
            Box::pin(async move {
                *provider.listener.lock().unwrap() = Some(listener);
                provider.listener_ready.notify_one();
                Ok(())
            })
        }

        fn get(&self, _data_id: String, _group: String) -> ConfigFuture<ConfigDocument> {
            let receiver = self.fetch_release.lock().unwrap().take().unwrap();
            Box::pin(async move {
                let _ = receiver.await;
                Ok(ConfigDocument::new(
                    ConfigFormat::Toml,
                    "value = 'stale-fetch'",
                ))
            })
        }

        fn remove_listener(
            &self,
            _data_id: String,
            _group: String,
            _listener: Arc<dyn ConfigChangeListener>,
        ) -> ConfigFuture<()> {
            let removals = self.removals.clone();
            Box::pin(async move {
                removals.fetch_add(1, Ordering::SeqCst);
                Ok(())
            })
        }
    }

    fn controlled_source() -> (NacosConfigSource, ControlledConfig, oneshot::Sender<()>) {
        let (fetch_sender, fetch_receiver) = oneshot::channel();
        let provider = ControlledConfig {
            listener_ready: Arc::new(Notify::new()),
            listener: Arc::new(Mutex::new(None)),
            fetch_release: Arc::new(Mutex::new(Some(fetch_receiver))),
            removals: Arc::new(AtomicUsize::new(0)),
        };
        (
            NacosConfigSource {
                client: Arc::new(provider.clone()),
            },
            provider,
            fetch_sender,
        )
    }

    #[tokio::test]
    async fn listener_update_wins_over_an_older_initial_fetch() {
        let (source, provider, fetch_sender) = controlled_source();
        let handle = source
            .prepare(ConfigKey::new("settings.toml").unwrap())
            .unwrap();
        let activation = tokio::spawn({
            let handle = handle.clone();
            async move { handle.activate().await }
        });
        provider.listener_ready.notified().await;
        provider
            .listener
            .lock()
            .unwrap()
            .as_ref()
            .unwrap()
            .notify(response(
                "settings.toml",
                "toml",
                "value = 'listener-update'",
            ));
        fetch_sender.send(()).unwrap();
        activation.await.unwrap().unwrap();

        let hot = handle.typed::<Demo>().unwrap();
        assert_eq!(hot.current().value, "listener-update");
        hot.close().await.unwrap();
        assert_eq!(provider.removals.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn cancelled_activation_waiter_still_removes_the_listener_once() {
        let (source, provider, fetch_sender) = controlled_source();
        let handle = source
            .prepare(ConfigKey::new("settings.toml").unwrap())
            .unwrap();
        let waiter = tokio::spawn({
            let handle = handle.clone();
            async move { handle.activate().await }
        });
        provider.listener_ready.notified().await;
        waiter.abort();
        assert!(waiter.await.unwrap_err().is_cancelled());

        let closing = tokio::spawn({
            let handle = handle.clone();
            async move { handle.close().await }
        });
        fetch_sender.send(()).unwrap();
        closing.await.unwrap().unwrap();
        assert_eq!(provider.removals.load(Ordering::SeqCst), 1);
        assert_eq!(
            handle.activate().await.unwrap_err().kind(),
            ConfigErrorKind::Cancelled
        );
    }
}
