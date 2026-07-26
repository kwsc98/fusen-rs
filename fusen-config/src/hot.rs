use crate::{ConfigError, ConfigErrorKind, ConfigOperation, parse};
use futures_util::FutureExt;
use serde::de::DeserializeOwned;
use std::{
    fmt,
    future::Future,
    pin::Pin,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::Instant,
};
use tokio::{runtime::Handle, sync::watch};

const MAX_KEY_BYTES: usize = 256;

/// Supported configuration serialization formats.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum ConfigFormat {
    /// TOML configuration.
    Toml,
    /// YAML configuration.
    Yaml,
}

impl ConfigFormat {
    /// Resolves a file extension, short format name, or common media type.
    pub fn from_name(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "toml" | "text/toml" | "application/toml" => Some(Self::Toml),
            "yaml" | "yml" | "text/yaml" | "application/yaml" | "application/x-yaml" => {
                Some(Self::Yaml)
            }
            _ => None,
        }
    }

    /// Returns the stable short format name.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Toml => "toml",
            Self::Yaml => "yaml",
        }
    }
}

impl fmt::Display for ConfigFormat {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// Raw provider content with an explicit serialization format.
#[derive(Clone, PartialEq, Eq)]
pub struct ConfigDocument {
    format: ConfigFormat,
    content: String,
}

impl ConfigDocument {
    /// Creates a raw configuration document.
    pub fn new(format: ConfigFormat, content: impl Into<String>) -> Self {
        Self {
            format,
            content: content.into(),
        }
    }

    /// Returns the declared serialization format.
    pub const fn format(&self) -> ConfigFormat {
        self.format
    }

    /// Returns the raw configuration content.
    pub fn content(&self) -> &str {
        &self.content
    }
}

impl fmt::Debug for ConfigDocument {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ConfigDocument")
            .field("format", &self.format)
            .field("content_bytes", &self.content.len())
            .finish()
    }
}

/// Provider-independent identity for one configuration resource.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ConfigKey {
    name: String,
    group: Option<String>,
}

impl ConfigKey {
    /// Creates a validated ungrouped configuration key.
    pub fn new(name: impl Into<String>) -> Result<Self, ConfigError> {
        Self::builder(name).build()
    }

    /// Starts a key builder.
    pub fn builder(name: impl Into<String>) -> ConfigKeyBuilder {
        ConfigKeyBuilder {
            name: name.into(),
            group: None,
        }
    }

    /// Returns the provider resource name.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Returns the optional provider group.
    pub fn group(&self) -> Option<&str> {
        self.group.as_deref()
    }
}

/// Builder for a validated [`ConfigKey`].
#[derive(Clone, Debug)]
pub struct ConfigKeyBuilder {
    name: String,
    group: Option<String>,
}

impl ConfigKeyBuilder {
    /// Sets the provider group.
    pub fn group(mut self, group: impl Into<String>) -> Self {
        self.group = Some(group.into());
        self
    }

    /// Validates and builds the key.
    pub fn build(self) -> Result<ConfigKey, ConfigError> {
        validate_key_component(&self.name, "configuration name")?;
        if let Some(group) = &self.group {
            validate_key_component(group, "configuration group")?;
        }
        Ok(ConfigKey {
            name: self.name,
            group: self.group,
        })
    }
}

fn validate_key_component(value: &str, field: &'static str) -> Result<(), ConfigError> {
    if !value.is_empty()
        && value.len() <= MAX_KEY_BYTES
        && value.trim() == value
        && !value.bytes().any(|byte| byte.is_ascii_control())
    {
        Ok(())
    } else {
        Err(ConfigError::message(
            ConfigOperation::Prepare,
            ConfigErrorKind::InvalidInput,
            format!(
                "{field} must be 1-{MAX_KEY_BYTES} bytes without surrounding whitespace or control characters"
            ),
        ))
    }
}

/// Owned, sendable future returned by configuration lifecycle APIs.
pub type ConfigFuture<T> = Pin<Box<dyn Future<Output = Result<T, ConfigError>> + Send + 'static>>;

/// Pluggable source that prepares ownership before any remote side effect starts.
pub trait ConfigSource: Send + Sync {
    /// Prepares one resource without fetching it or installing a listener yet.
    fn prepare(&self, key: ConfigKey) -> Result<ConfigHandle, ConfigError>;
}

/// Provider-owned publication access for a prepared hot configuration.
///
/// This type exposes replacement operations only. Its Tokio channel and lifecycle state remain
/// private to `fusen-config`.
#[derive(Clone)]
pub struct ConfigPublisher {
    inner: Arc<PublisherInner>,
}

impl fmt::Debug for ConfigPublisher {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ConfigPublisher")
            .finish_non_exhaustive()
    }
}

/// Creates a prepared handle from provider-owned activation and cleanup operations.
///
/// The factory runs synchronously and must only construct owned state. Neither returned future is
/// polled before [`ConfigHandle::activate`]. The publisher may be cloned into a provider callback;
/// publications after close are rejected.
pub fn prepare_config<P, A, C, CF>(prepare: P) -> ConfigHandle
where
    P: FnOnce(ConfigPublisher) -> (A, C),
    A: Future<Output = Result<ConfigDocument, ConfigError>> + Send + 'static,
    C: FnOnce() -> CF + Send + 'static,
    CF: Future<Output = Result<(), ConfigError>> + Send + 'static,
{
    let publisher = ConfigPublisher::new();
    let (activate, close) = prepare(publisher.clone());
    ConfigHandle {
        lifecycle: Arc::new(Lifecycle::new(
            Box::pin(activate),
            Box::new(move || Box::pin(close())),
            publisher,
        )),
    }
}

/// Prepared ownership of one provider listener and its latest raw document.
///
/// Clones share one activation and cleanup terminal result. Cancelling an activation or close
/// waiter never cancels provider work. Dropping the final clone requests cleanup without blocking.
#[derive(Clone)]
pub struct ConfigHandle {
    lifecycle: Arc<Lifecycle>,
}

impl ConfigHandle {
    /// Starts provider activation once and shares its terminal result with every caller.
    pub fn activate(&self) -> ConfigFuture<()> {
        let lifecycle = self.lifecycle.clone();
        Box::pin(async move {
            lifecycle.ensure_started()?;
            lifecycle.wait_activation().await
        })
    }

    /// Builds a typed last-good view after successful activation.
    ///
    /// Provider updates are parsed in a background task. Invalid updates are reported through
    /// [`HotConfig::last_error`] and never replace the most recent valid typed value.
    pub fn typed<T>(&self) -> Result<HotConfig<T>, ConfigError>
    where
        T: DeserializeOwned + Send + Sync + 'static,
    {
        match self.lifecycle.activation_result.borrow().clone() {
            Some(Ok(())) => {}
            Some(Err(error)) => return Err(error),
            None => {
                return Err(ConfigError::message(
                    ConfigOperation::Watch,
                    ConfigErrorKind::InvalidInput,
                    "configuration handle must be activated before creating a typed view",
                ));
            }
        }

        let (baseline, current, mut raw_receiver) = self.lifecycle.publisher.inputs()?;
        if current.closed {
            return Err(ConfigError::message(
                ConfigOperation::Watch,
                ConfigErrorKind::Cancelled,
                "configuration handle is closed",
            ));
        }
        let mut value = None;
        let mut last_error = None;

        if let Some(document) = baseline.as_ref() {
            match parse(document) {
                Ok(parsed) => value = Some(parsed),
                Err(error) => last_error = Some(error),
            }
        }
        if let Some(document) = current.document.as_ref()
            && baseline.as_ref() != Some(document)
        {
            match parse(document) {
                Ok(parsed) => {
                    value = Some(parsed);
                    last_error = None;
                }
                Err(error) => last_error = Some(error),
            }
        }
        let value = value.ok_or_else(|| {
            last_error.clone().unwrap_or_else(|| {
                ConfigError::message(
                    ConfigOperation::Watch,
                    ConfigErrorKind::Internal,
                    "activated configuration source did not publish an initial document",
                )
            })
        })?;

        let runtime = Handle::try_current().map_err(|error| {
            ConfigError::new(ConfigOperation::Watch, ConfigErrorKind::Internal, error)
        })?;
        let initial = ConfigSnapshot {
            revision: 1,
            observed_at: Instant::now(),
            value: Arc::new(value),
        };
        let (sender, receiver) = watch::channel(initial);
        let (error_sender, error_receiver) = watch::channel(last_error);
        runtime.spawn(async move {
            let mut revision = 1_u64;
            loop {
                tokio::select! {
                    _ = sender.closed() => break,
                    changed = raw_receiver.changed() => {
                        if changed.is_err() {
                            break;
                        }
                        let snapshot = raw_receiver.borrow_and_update().clone();
                        if snapshot.closed {
                            break;
                        }
                        let Some(document) = snapshot.document else {
                            continue;
                        };
                        match parse(&document) {
                            Ok(value) => {
                                let Some(next_revision) = revision.checked_add(1) else {
                                    error_sender.send_replace(Some(ConfigError::message(
                                        ConfigOperation::Watch,
                                        ConfigErrorKind::Internal,
                                        "typed configuration revision overflowed",
                                    )));
                                    break;
                                };
                                revision = next_revision;
                                error_sender.send_replace(None);
                                sender.send_replace(ConfigSnapshot {
                                    revision,
                                    observed_at: Instant::now(),
                                    value: Arc::new(value),
                                });
                            }
                            Err(error) => {
                                tracing::warn!(
                                    error = %error,
                                    format = %document.format(),
                                    "hot configuration update rejected; retaining last-good value"
                                );
                                error_sender.send_replace(Some(error));
                            }
                        }
                    }
                }
            }
        });

        Ok(HotConfig {
            handle: self.clone(),
            receiver,
            error_receiver,
        })
    }

    /// Requests cleanup without waiting for provider completion.
    pub fn request_close(&self) {
        self.lifecycle.request_close();
    }

    /// Requests cleanup and shares its terminal result with every caller.
    pub fn close(&self) -> ConfigFuture<()> {
        let lifecycle = self.lifecycle.clone();
        Box::pin(async move {
            lifecycle.request_close();
            lifecycle.wait_close().await
        })
    }
}

impl fmt::Debug for ConfigHandle {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ConfigHandle")
            .finish_non_exhaustive()
    }
}

/// One successfully parsed, monotonically versioned configuration snapshot.
pub struct ConfigSnapshot<T> {
    revision: u64,
    observed_at: Instant,
    value: Arc<T>,
}

impl<T> ConfigSnapshot<T> {
    /// Returns the process-local revision of this successfully parsed value.
    pub const fn revision(&self) -> u64 {
        self.revision
    }

    /// Returns when this value was accepted as the last-good snapshot.
    pub const fn observed_at(&self) -> Instant {
        self.observed_at
    }

    /// Returns the shared typed value.
    pub fn value(&self) -> &Arc<T> {
        &self.value
    }

    /// Clones the shared typed value without copying `T`.
    pub fn shared_value(&self) -> Arc<T> {
        self.value.clone()
    }
}

impl<T> Clone for ConfigSnapshot<T> {
    fn clone(&self) -> Self {
        Self {
            revision: self.revision,
            observed_at: self.observed_at,
            value: self.value.clone(),
        }
    }
}

impl<T: fmt::Debug> fmt::Debug for ConfigSnapshot<T> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ConfigSnapshot")
            .field("revision", &self.revision)
            .field("observed_at", &self.observed_at)
            .field("value", &self.value)
            .finish()
    }
}

/// Cloneable latest-wins view that only publishes successfully parsed values.
pub struct HotConfig<T> {
    handle: ConfigHandle,
    receiver: watch::Receiver<ConfigSnapshot<T>>,
    error_receiver: watch::Receiver<Option<ConfigError>>,
}

impl<T> HotConfig<T> {
    /// Returns the latest successfully parsed snapshot.
    pub fn snapshot(&self) -> ConfigSnapshot<T> {
        self.receiver.borrow().clone()
    }

    /// Returns the latest successfully parsed value.
    pub fn current(&self) -> Arc<T> {
        self.snapshot().shared_value()
    }

    /// Returns the most recent rejected provider update, if any.
    pub fn last_error(&self) -> Option<ConfigError> {
        self.error_receiver.borrow().clone()
    }

    /// Waits for and returns the next successfully parsed value.
    pub async fn changed(&mut self) -> Result<ConfigSnapshot<T>, ConfigError> {
        self.receiver.changed().await.map_err(|_| {
            ConfigError::message(
                ConfigOperation::Watch,
                ConfigErrorKind::Cancelled,
                "configuration source closed",
            )
        })?;
        Ok(self.snapshot())
    }

    /// Explicitly closes the provider listener and waits for its shared terminal result.
    pub fn close(&self) -> ConfigFuture<()> {
        self.handle.close()
    }

    /// Returns the prepared lifecycle handle retained by this typed view.
    pub fn handle(&self) -> ConfigHandle {
        self.handle.clone()
    }
}

impl<T> Clone for HotConfig<T> {
    fn clone(&self) -> Self {
        Self {
            handle: self.handle.clone(),
            receiver: self.receiver.clone(),
            error_receiver: self.error_receiver.clone(),
        }
    }
}

impl<T> fmt::Debug for HotConfig<T> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("HotConfig")
            .field("revision", &self.receiver.borrow().revision())
            .finish_non_exhaustive()
    }
}

#[derive(Clone, Debug)]
struct RawSnapshot {
    document: Option<ConfigDocument>,
    closed: bool,
}

struct PublisherInner {
    state: Mutex<PublisherState>,
}

struct PublisherState {
    revision: u64,
    baseline: Option<ConfigDocument>,
    closed: bool,
    sender: watch::Sender<RawSnapshot>,
}

impl ConfigPublisher {
    fn new() -> Self {
        let (sender, _) = watch::channel(RawSnapshot {
            document: None,
            closed: false,
        });
        Self {
            inner: Arc::new(PublisherInner {
                state: Mutex::new(PublisherState {
                    revision: 0,
                    baseline: None,
                    closed: false,
                    sender,
                }),
            }),
        }
    }

    /// Replaces the latest raw provider document.
    pub fn publish(&self, document: ConfigDocument) -> Result<(), ConfigError> {
        let mut state = self.lock_state(ConfigOperation::Publish)?;
        if state.closed {
            return Err(ConfigError::message(
                ConfigOperation::Publish,
                ConfigErrorKind::Cancelled,
                "configuration publisher is closed",
            ));
        }
        state.revision = state.revision.checked_add(1).ok_or_else(|| {
            ConfigError::message(
                ConfigOperation::Publish,
                ConfigErrorKind::Internal,
                "raw configuration revision overflowed",
            )
        })?;
        state.sender.send_replace(RawSnapshot {
            document: Some(document),
            closed: false,
        });
        Ok(())
    }

    fn publish_initial(&self, document: ConfigDocument) -> Result<(), ConfigError> {
        let mut state = self.lock_state(ConfigOperation::Activate)?;
        if state.closed {
            return Err(ConfigError::message(
                ConfigOperation::Activate,
                ConfigErrorKind::Cancelled,
                "configuration publisher closed during activation",
            ));
        }
        state.baseline = Some(document.clone());
        if state.sender.borrow().document.is_none() {
            state.revision = state.revision.checked_add(1).ok_or_else(|| {
                ConfigError::message(
                    ConfigOperation::Activate,
                    ConfigErrorKind::Internal,
                    "raw configuration revision overflowed",
                )
            })?;
            state.sender.send_replace(RawSnapshot {
                document: Some(document),
                closed: false,
            });
        }
        Ok(())
    }

    fn inputs(
        &self,
    ) -> Result<
        (
            Option<ConfigDocument>,
            RawSnapshot,
            watch::Receiver<RawSnapshot>,
        ),
        ConfigError,
    > {
        let state = self.lock_state(ConfigOperation::Watch)?;
        let mut receiver = state.sender.subscribe();
        let current = receiver.borrow_and_update().clone();
        Ok((state.baseline.clone(), current, receiver))
    }

    fn finish(&self) {
        let Ok(mut state) = self.inner.state.lock() else {
            return;
        };
        finish_publisher(&mut state);
    }

    fn lock_state(
        &self,
        operation: ConfigOperation,
    ) -> Result<std::sync::MutexGuard<'_, PublisherState>, ConfigError> {
        self.inner.state.lock().map_err(|_| {
            ConfigError::message(
                operation,
                ConfigErrorKind::Internal,
                "configuration publisher lock was poisoned",
            )
        })
    }
}

fn finish_publisher(state: &mut PublisherState) {
    if state.closed {
        return;
    }
    state.closed = true;
    state.revision = state.revision.saturating_add(1);
    let document = state.sender.borrow().document.clone();
    state.sender.send_replace(RawSnapshot {
        document,
        closed: true,
    });
}

impl Drop for PublisherInner {
    fn drop(&mut self) {
        if let Ok(state) = self.state.get_mut() {
            finish_publisher(state);
        }
    }
}

type ActivationFuture = ConfigFuture<ConfigDocument>;
type CloseFactory = Box<dyn FnOnce() -> ConfigFuture<()> + Send + 'static>;
type SharedResult = Option<Result<(), ConfigError>>;

struct PreparedLifecycle {
    activate: ActivationFuture,
    close: CloseFactory,
}

enum StartState {
    Prepared(Option<PreparedLifecycle>),
    Started,
    Finished,
}

struct Lifecycle {
    start: Mutex<StartState>,
    close_requested: AtomicBool,
    close_request: watch::Sender<bool>,
    activation_result: watch::Sender<SharedResult>,
    close_result: watch::Sender<SharedResult>,
    publisher: ConfigPublisher,
}

impl Lifecycle {
    fn new(activate: ActivationFuture, close: CloseFactory, publisher: ConfigPublisher) -> Self {
        let (close_request, _) = watch::channel(false);
        let (activation_result, _) = watch::channel(None);
        let (close_result, _) = watch::channel(None);
        Self {
            start: Mutex::new(StartState::Prepared(Some(PreparedLifecycle {
                activate,
                close,
            }))),
            close_requested: AtomicBool::new(false),
            close_request,
            activation_result,
            close_result,
            publisher,
        }
    }

    fn ensure_started(&self) -> Result<(), ConfigError> {
        if self.activation_result.borrow().is_some() {
            return Ok(());
        }
        let runtime = match Handle::try_current() {
            Ok(runtime) => runtime,
            Err(error) => {
                let error =
                    ConfigError::new(ConfigOperation::Activate, ConfigErrorKind::Internal, error);
                self.finish_before_start(Err(error.clone()));
                return Err(error);
            }
        };
        let mut start = self.start.lock().unwrap_or_else(|error| error.into_inner());
        match &mut *start {
            StartState::Started | StartState::Finished => return Ok(()),
            StartState::Prepared(_) => {}
        }
        if self.close_requested.load(Ordering::Acquire) {
            let prepared = match &mut *start {
                StartState::Prepared(prepared) => prepared.take(),
                StartState::Started | StartState::Finished => None,
            };
            drop(prepared);
            *start = StartState::Finished;
            self.publish_pre_activation_close();
            return Ok(());
        }
        let prepared = match &mut *start {
            StartState::Prepared(prepared) => prepared
                .take()
                .expect("prepared lifecycle is present before activation"),
            StartState::Started | StartState::Finished => unreachable!(),
        };
        runtime.spawn(
            LifecycleWorker {
                activate: Some(prepared.activate),
                close: Some(prepared.close),
                close_request: self.close_request.subscribe(),
                activation_result: self.activation_result.clone(),
                close_result: self.close_result.clone(),
                publisher: self.publisher.clone(),
                activation_published: false,
                close_published: false,
            }
            .run(),
        );
        *start = StartState::Started;
        Ok(())
    }

    fn request_close(&self) {
        if !self.close_requested.swap(true, Ordering::AcqRel) {
            self.close_request.send_replace(true);
        }
        let mut start = self.start.lock().unwrap_or_else(|error| error.into_inner());
        let prepared = match &mut *start {
            StartState::Prepared(prepared) => prepared.take(),
            StartState::Started | StartState::Finished => return,
        };
        drop(prepared);
        *start = StartState::Finished;
        self.publish_pre_activation_close();
    }

    fn finish_before_start(&self, activation: Result<(), ConfigError>) {
        let mut start = self.start.lock().unwrap_or_else(|error| error.into_inner());
        let prepared = match &mut *start {
            StartState::Prepared(prepared) => prepared.take(),
            StartState::Started | StartState::Finished => return,
        };
        drop(prepared);
        *start = StartState::Finished;
        self.publisher.finish();
        self.activation_result.send_replace(Some(activation));
        self.close_result.send_replace(Some(Ok(())));
    }

    fn publish_pre_activation_close(&self) {
        self.publisher.finish();
        self.activation_result
            .send_replace(Some(Err(ConfigError::message(
                ConfigOperation::Activate,
                ConfigErrorKind::Cancelled,
                "configuration handle closed before activation",
            ))));
        self.close_result.send_replace(Some(Ok(())));
    }

    async fn wait_activation(&self) -> Result<(), ConfigError> {
        wait_for_result(
            self.activation_result.subscribe(),
            ConfigOperation::Activate,
            "activation worker ended without a result",
        )
        .await
    }

    async fn wait_close(&self) -> Result<(), ConfigError> {
        wait_for_result(
            self.close_result.subscribe(),
            ConfigOperation::Close,
            "cleanup worker ended without a result",
        )
        .await
    }
}

impl Drop for Lifecycle {
    fn drop(&mut self) {
        self.request_close();
    }
}

struct LifecycleWorker {
    activate: Option<ActivationFuture>,
    close: Option<CloseFactory>,
    close_request: watch::Receiver<bool>,
    activation_result: watch::Sender<SharedResult>,
    close_result: watch::Sender<SharedResult>,
    publisher: ConfigPublisher,
    activation_published: bool,
    close_published: bool,
}

impl LifecycleWorker {
    async fn run(mut self) {
        let activation = self
            .activate
            .take()
            .expect("activation future is present until the worker starts");
        let activation = match std::panic::AssertUnwindSafe(activation)
            .catch_unwind()
            .await
        {
            Ok(Ok(document)) => self.publisher.publish_initial(document),
            Ok(Err(error)) => Err(error),
            Err(_) => Err(ConfigError::message(
                ConfigOperation::Activate,
                ConfigErrorKind::Internal,
                "configuration provider activation panicked",
            )),
        };
        let activation = if *self.close_request.borrow() && activation.is_ok() {
            Err(ConfigError::message(
                ConfigOperation::Activate,
                ConfigErrorKind::Cancelled,
                "configuration handle closed while activation was pending",
            ))
        } else {
            activation
        };
        self.activation_result.send_replace(Some(activation));
        self.activation_published = true;

        if !*self.close_request.borrow() {
            let _ = self.close_request.changed().await;
        }
        let close = self
            .close
            .take()
            .expect("cleanup factory is present until close is requested");
        let close = match std::panic::catch_unwind(std::panic::AssertUnwindSafe(close)) {
            Ok(close) => match std::panic::AssertUnwindSafe(close).catch_unwind().await {
                Ok(result) => result,
                Err(_) => Err(ConfigError::message(
                    ConfigOperation::Close,
                    ConfigErrorKind::Internal,
                    "configuration provider cleanup panicked",
                )),
            },
            Err(_) => Err(ConfigError::message(
                ConfigOperation::Close,
                ConfigErrorKind::Internal,
                "configuration provider cleanup factory panicked",
            )),
        };
        self.publisher.finish();
        self.close_result.send_replace(Some(close));
        self.close_published = true;
    }
}

impl Drop for LifecycleWorker {
    fn drop(&mut self) {
        self.publisher.finish();
        if !self.activation_published {
            self.activation_result
                .send_replace(Some(Err(ConfigError::message(
                    ConfigOperation::Activate,
                    ConfigErrorKind::Internal,
                    "configuration activation worker was aborted",
                ))));
        }
        if !self.close_published {
            self.close_result
                .send_replace(Some(Err(ConfigError::message(
                    ConfigOperation::Close,
                    ConfigErrorKind::CleanupAborted,
                    "configuration cleanup worker was aborted",
                ))));
        }
    }
}

async fn wait_for_result(
    mut result: watch::Receiver<SharedResult>,
    operation: ConfigOperation,
    ended_message: &'static str,
) -> Result<(), ConfigError> {
    loop {
        if let Some(result) = result.borrow().clone() {
            return result;
        }
        result.changed().await.map_err(|_| {
            ConfigError::message(operation, ConfigErrorKind::Internal, ended_message)
        })?;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::Deserialize;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use tokio::sync::{Notify, oneshot};

    fn document(value: &str) -> ConfigDocument {
        ConfigDocument::new(ConfigFormat::Toml, format!("value = '{value}'"))
    }

    #[derive(Debug, Deserialize)]
    struct Demo {
        value: String,
    }

    #[test]
    fn config_keys_are_bounded_and_trimmed() {
        assert!(ConfigKey::new("application.toml").is_ok());
        assert!(ConfigKey::new("").is_err());
        assert!(ConfigKey::new(" bad").is_err());
        assert!(ConfigKey::builder("app").group("prod").build().is_ok());
    }

    #[tokio::test]
    async fn prepared_handle_has_no_effect_before_activation() {
        let activations = Arc::new(AtomicUsize::new(0));
        let cleanups = Arc::new(AtomicUsize::new(0));
        let handle = prepare_config({
            let activations = activations.clone();
            let cleanups = cleanups.clone();
            move |_| {
                (
                    async move {
                        activations.fetch_add(1, Ordering::SeqCst);
                        Ok(document("active"))
                    },
                    move || async move {
                        cleanups.fetch_add(1, Ordering::SeqCst);
                        Ok(())
                    },
                )
            }
        });

        tokio::task::yield_now().await;
        assert_eq!(activations.load(Ordering::SeqCst), 0);
        handle.close().await.unwrap();
        assert_eq!(activations.load(Ordering::SeqCst), 0);
        assert_eq!(cleanups.load(Ordering::SeqCst), 0);
        assert_eq!(
            handle.activate().await.unwrap_err().kind(),
            ConfigErrorKind::Cancelled
        );
    }

    #[tokio::test]
    async fn invalid_updates_retain_the_last_good_value() {
        let publisher_slot = Arc::new(Mutex::new(None));
        let handle = prepare_config({
            let publisher_slot = publisher_slot.clone();
            move |publisher| {
                *publisher_slot.lock().unwrap() = Some(publisher);
                (async { Ok(document("initial")) }, || async { Ok(()) })
            }
        });
        handle.activate().await.unwrap();
        let mut hot = handle.typed::<Demo>().unwrap();
        let publisher = publisher_slot.lock().unwrap().clone().unwrap();

        publisher
            .publish(ConfigDocument::new(ConfigFormat::Toml, "value = ["))
            .unwrap();
        hot.error_receiver.changed().await.unwrap();
        assert_eq!(hot.current().value, "initial");
        assert_eq!(
            hot.last_error().unwrap().kind(),
            ConfigErrorKind::InvalidData
        );

        publisher.publish(document("recovered")).unwrap();
        assert_eq!(hot.changed().await.unwrap().value().value, "recovered");
        assert!(hot.last_error().is_none());
        hot.close().await.unwrap();
    }

    #[tokio::test]
    async fn cancelling_activation_waiter_does_not_cancel_provider_work() {
        let started = Arc::new(Notify::new());
        let (release_sender, release_receiver) = oneshot::channel();
        let cleanups = Arc::new(AtomicUsize::new(0));
        let handle = prepare_config({
            let started = started.clone();
            let cleanups = cleanups.clone();
            move |_| {
                (
                    async move {
                        started.notify_one();
                        let _ = release_receiver.await;
                        Ok(document("late"))
                    },
                    move || async move {
                        cleanups.fetch_add(1, Ordering::SeqCst);
                        Ok(())
                    },
                )
            }
        });
        let waiter = tokio::spawn({
            let handle = handle.clone();
            async move { handle.activate().await }
        });
        started.notified().await;
        waiter.abort();
        assert!(waiter.await.unwrap_err().is_cancelled());

        let closing = tokio::spawn({
            let handle = handle.clone();
            async move { handle.close().await }
        });
        release_sender.send(()).unwrap();
        closing.await.unwrap().unwrap();
        assert_eq!(cleanups.load(Ordering::SeqCst), 1);
        assert_eq!(
            handle.activate().await.unwrap_err().kind(),
            ConfigErrorKind::Cancelled
        );
    }

    #[tokio::test]
    async fn dropping_the_last_waiter_compensates_a_late_success() {
        let started = Arc::new(Notify::new());
        let (release_sender, release_receiver) = oneshot::channel();
        let cleaned = Arc::new(Notify::new());
        let cleanups = Arc::new(AtomicUsize::new(0));
        let handle = prepare_config({
            let started = started.clone();
            let cleaned = cleaned.clone();
            let cleanups = cleanups.clone();
            move |_| {
                (
                    async move {
                        started.notify_one();
                        let _ = release_receiver.await;
                        Ok(document("late"))
                    },
                    move || async move {
                        cleanups.fetch_add(1, Ordering::SeqCst);
                        cleaned.notify_one();
                        Ok(())
                    },
                )
            }
        });
        let waiter = tokio::spawn(async move { handle.activate().await });
        started.notified().await;
        waiter.abort();
        assert!(waiter.await.unwrap_err().is_cancelled());

        release_sender.send(()).unwrap();
        cleaned.notified().await;
        assert_eq!(cleanups.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn concurrent_close_waiters_share_one_cleanup() {
        let cleanups = Arc::new(AtomicUsize::new(0));
        let handle = prepare_config({
            let cleanups = cleanups.clone();
            move |_| {
                (async { Ok(document("active")) }, move || async move {
                    cleanups.fetch_add(1, Ordering::SeqCst);
                    Ok(())
                })
            }
        });
        handle.activate().await.unwrap();
        let first = tokio::spawn({
            let handle = handle.clone();
            async move { handle.close().await }
        });
        let second = tokio::spawn({
            let handle = handle.clone();
            async move { handle.close().await }
        });
        first.await.unwrap().unwrap();
        second.await.unwrap().unwrap();
        assert_eq!(cleanups.load(Ordering::SeqCst), 1);
        assert_eq!(
            handle.typed::<Demo>().unwrap_err().kind(),
            ConfigErrorKind::Cancelled
        );
    }
}
