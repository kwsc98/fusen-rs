use super::{config::DiscoveryConfig, endpoint_breakers::EndpointBreakers};
use crate::{
    ClientError, ClientErrorKind, resilience::retry::full_jitter_backoff,
    runtime::metrics::SafeMetrics,
};
use fusen_contract::ServiceSelector;
use fusen_observability::{DirectoryMetricState, DirectoryStateChangedEvent, MetricEvent};
use fusen_register::{
    Registry, SubscriptionHandle, SubscriptionRequest,
    directory::{Directory, DirectoryPublisher, DirectoryState, directory},
    error::{RegistryError, RegistryErrorKind, RegistryOperation},
};
use std::{
    collections::HashMap,
    future::Future,
    panic::{AssertUnwindSafe, catch_unwind},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};
use tokio::sync::watch;
use tokio_util::sync::CancellationToken;

#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
struct SubscriptionKey {
    selector: ServiceSelector,
}

#[derive(Clone)]
enum SlotState {
    Initializing,
    Ready,
    Stale,
    Unavailable,
    Failed(RegistryError),
    Quarantined,
    Closed,
}

struct Slot {
    directory: Directory,
    retained_publisher: Arc<Mutex<Option<DirectoryPublisher>>>,
    state: watch::Receiver<SlotState>,
    completion: watch::Receiver<bool>,
}

pub(crate) struct SubscriptionManager {
    registry: Arc<dyn Registry>,
    config: DiscoveryConfig,
    slots: Mutex<HashMap<SubscriptionKey, Arc<Slot>>>,
    closed: AtomicBool,
    shutdown: CancellationToken,
    metrics: SafeMetrics,
    endpoint_breakers: EndpointBreakers,
}

impl SubscriptionManager {
    pub(crate) fn new(
        registry: Arc<dyn Registry>,
        config: DiscoveryConfig,
        metrics: SafeMetrics,
        endpoint_breakers: EndpointBreakers,
    ) -> Arc<Self> {
        Arc::new(Self {
            registry,
            config,
            slots: Mutex::new(HashMap::new()),
            closed: AtomicBool::new(false),
            shutdown: CancellationToken::new(),
            metrics,
            endpoint_breakers,
        })
    }

    pub(crate) async fn acquire(
        self: &Arc<Self>,
        selector: ServiceSelector,
    ) -> Result<Directory, ClientError> {
        if self.closed.load(Ordering::Acquire) {
            return Err(closed());
        }
        let key = SubscriptionKey { selector };
        let slot = {
            let mut slots = self.slots.lock().unwrap_or_else(|error| error.into_inner());
            if self.closed.load(Ordering::Acquire) {
                return Err(closed());
            }
            if let Some(slot) = slots.get(&key) {
                slot.clone()
            } else {
                if slots.len() >= self.config.max_subscriptions() {
                    return Err(ClientError::from_message(
                        ClientErrorKind::Discovery,
                        "runtime subscription limit reached",
                    ));
                }
                let (publisher, directory) = directory();
                let (state_sender, state) = watch::channel(SlotState::Initializing);
                let (completion_sender, completion) = watch::channel(false);
                let supervisor_directory = directory.clone();
                let retained_publisher = Arc::new(Mutex::new(Some(publisher.clone())));
                let slot = Arc::new(Slot {
                    directory,
                    retained_publisher: retained_publisher.clone(),
                    state,
                    completion,
                });
                tokio::spawn(run_subscription(
                    self.registry.clone(),
                    key.clone(),
                    self.config.clone(),
                    self.shutdown.child_token(),
                    publisher,
                    supervisor_directory,
                    retained_publisher,
                    state_sender,
                    completion_sender,
                    self.metrics.clone(),
                    self.endpoint_breakers.clone(),
                ));
                slots.insert(key, slot.clone());
                slot
            }
        };
        wait_until_ready(slot, self.config.initial_timeout()).await
    }

    pub(crate) fn begin_shutdown(&self) {
        let _slots = self.slots.lock().unwrap_or_else(|error| error.into_inner());
        if !self.closed.swap(true, Ordering::AcqRel) {
            self.shutdown.cancel();
        }
    }

    pub(crate) async fn closed(&self) -> Result<(), RegistryError> {
        let mut slots = self
            .slots
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .iter()
            .map(|(key, slot)| (key.clone(), slot.clone()))
            .collect::<Vec<_>>();
        slots.sort_by(|(left, _), (right, _)| left.cmp(right));
        let outcomes = futures_util::future::join_all(
            slots
                .into_iter()
                .map(|(_, slot)| wait_for_slot_completion(slot)),
        )
        .await;
        let mut first_error = None;
        for outcome in outcomes {
            if let Err(error) = outcome
                && first_error.is_none()
            {
                first_error = Some(error);
            }
        }
        first_error.map_or(Ok(()), Err)
    }

    pub(crate) fn finish_shutdown(&self) {
        let (keys, publishers) = {
            let slots = self.slots.lock().unwrap_or_else(|error| error.into_inner());
            let keys = slots.keys().cloned().collect::<Vec<_>>();
            let publishers = slots
                .values()
                .filter_map(|slot| {
                    slot.retained_publisher
                        .lock()
                        .unwrap_or_else(|error| error.into_inner())
                        .take()
                })
                .collect::<Vec<_>>();
            (keys, publishers)
        };
        for key in keys {
            self.endpoint_breakers.remove_discovery(&key.selector);
        }
        drop(publishers);
    }
}

async fn wait_for_slot_completion(slot: Arc<Slot>) -> Result<(), RegistryError> {
    let mut completion = slot.completion.clone();
    while !*completion.borrow() {
        completion.changed().await.map_err(|_| {
            RegistryError::message(
                RegistryOperation::CloseSubscription,
                RegistryErrorKind::CleanupAborted,
                "subscription supervisor ended without a terminal result",
            )
        })?;
    }
    match slot.state.borrow().clone() {
        SlotState::Failed(error) => Err(error),
        _ => Ok(()),
    }
}

async fn wait_until_ready(slot: Arc<Slot>, timeout: Duration) -> Result<Directory, ClientError> {
    let mut state = slot.state.clone();
    tokio::time::timeout(timeout, async {
        loop {
            let current = state.borrow().clone();
            match current {
                SlotState::Ready | SlotState::Stale => return Ok(slot.directory.clone()),
                SlotState::Failed(error) => {
                    return Err(ClientError::with_source(
                        ClientErrorKind::Discovery,
                        "registry subscription failed",
                        error,
                    ));
                }
                SlotState::Quarantined => {
                    return Err(ClientError::from_message(
                        ClientErrorKind::Discovery,
                        "subscription is quarantined until provider cleanup completes",
                    ));
                }
                SlotState::Unavailable => {
                    return Err(ClientError::from_message(
                        ClientErrorKind::Discovery,
                        "service directory is unavailable",
                    ));
                }
                SlotState::Closed => return Err(closed()),
                SlotState::Initializing => {}
            }
            state.changed().await.map_err(|_| closed())?;
        }
    })
    .await
    .map_err(|_| {
        ClientError::from_message(
            ClientErrorKind::Timeout,
            "initial discovery did not become Ready before its deadline",
        )
    })?
}

#[derive(Clone, Copy)]
struct DirectoryForwarder<'a> {
    publisher: &'a DirectoryPublisher,
    directory: &'a Directory,
    state: &'a watch::Sender<SlotState>,
    metrics: &'a SafeMetrics,
    selector: &'a ServiceSelector,
    endpoint_breakers: &'a EndpointBreakers,
}

#[allow(clippy::too_many_arguments)]
async fn run_subscription(
    registry: Arc<dyn Registry>,
    key: SubscriptionKey,
    config: DiscoveryConfig,
    shutdown: CancellationToken,
    publisher: DirectoryPublisher,
    directory: Directory,
    retained_publisher: Arc<Mutex<Option<DirectoryPublisher>>>,
    state: watch::Sender<SlotState>,
    completion: watch::Sender<bool>,
    metrics: SafeMetrics,
    endpoint_breakers: EndpointBreakers,
) {
    record_directory_state(&metrics, &key.selector, DirectoryMetricState::Initializing);
    let forwarder = DirectoryForwarder {
        publisher: &publisher,
        directory: &directory,
        state: &state,
        metrics: &metrics,
        selector: &key.selector,
        endpoint_breakers: &endpoint_breakers,
    };
    let mut reconnect_attempt = 0u8;
    let mut stale_deadline = None;
    let mut terminal_state = SlotState::Closed;
    loop {
        if shutdown.is_cancelled() {
            break;
        }
        let prepared = catch_unwind(AssertUnwindSafe(|| {
            registry.prepare_subscription(SubscriptionRequest::new(key.selector.clone()))
        }));
        let handle = match prepared {
            Ok(Ok(handle)) => handle,
            Ok(Err(error)) => {
                set_provider_failure(forwarder, error);
                if !await_with_stale_deadline(
                    reconnect_delay(&shutdown, &config, &mut reconnect_attempt),
                    &mut stale_deadline,
                    forwarder,
                )
                .await
                {
                    break;
                }
                set_retrying_state(forwarder);
                continue;
            }
            Err(_) => {
                terminal_state = SlotState::Quarantined;
                state.send_replace(terminal_state.clone());
                record_directory_state(&metrics, &key.selector, DirectoryMetricState::Unavailable);
                break;
            }
        };
        let activated = await_with_stale_deadline(
            async {
                tokio::select! {
                    biased;
                    () = shutdown.cancelled() => None,
                    result = tokio::time::timeout(
                        config.operation_timeout(),
                        handle.activate(),
                    ) => Some(result),
                }
            },
            &mut stale_deadline,
            forwarder,
        )
        .await;
        let Some(activated) = activated else {
            if let Err(error) = await_with_stale_deadline(
                close_generation(&handle, &state, &config),
                &mut stale_deadline,
                forwarder,
            )
            .await
            {
                terminal_state = SlotState::Failed(error);
                state.send_replace(terminal_state.clone());
            }
            break;
        };
        let mut raw = match activated {
            Ok(Ok(directory)) => directory,
            Ok(Err(error)) => {
                set_provider_failure(forwarder, error);
                if let Err(error) = await_with_stale_deadline(
                    close_generation(&handle, &state, &config),
                    &mut stale_deadline,
                    forwarder,
                )
                .await
                {
                    terminal_state = SlotState::Failed(error);
                    state.send_replace(terminal_state.clone());
                    break;
                }
                if !await_with_stale_deadline(
                    reconnect_delay(&shutdown, &config, &mut reconnect_attempt),
                    &mut stale_deadline,
                    forwarder,
                )
                .await
                {
                    break;
                }
                set_retrying_state(forwarder);
                continue;
            }
            Err(_) => {
                mark_disconnected(forwarder, &mut stale_deadline, config.max_staleness());
                state.send_replace(SlotState::Quarantined);
                record_directory_state(&metrics, &key.selector, DirectoryMetricState::Unavailable);
                if let Err(error) = await_with_stale_deadline(
                    close_generation(&handle, &state, &config),
                    &mut stale_deadline,
                    forwarder,
                )
                .await
                {
                    terminal_state = SlotState::Failed(error);
                    state.send_replace(terminal_state.clone());
                    break;
                }
                if !await_with_stale_deadline(
                    reconnect_delay(&shutdown, &config, &mut reconnect_attempt),
                    &mut stale_deadline,
                    forwarder,
                )
                .await
                {
                    break;
                }
                set_retrying_state(forwarder);
                continue;
            }
        };
        reconnect_attempt = 0;
        update_snapshot(
            forwarder,
            raw.snapshot(),
            &mut stale_deadline,
            config.max_staleness(),
        );
        let provider_disconnected = loop {
            if let Some(deadline) = stale_deadline {
                tokio::select! {
                    biased;
                    () = shutdown.cancelled() => break false,
                    changed = raw.changed() => match changed {
                        Ok(snapshot) => {
                            update_snapshot(
                                forwarder,
                                snapshot,
                                &mut stale_deadline,
                                config.max_staleness(),
                            );
                        }
                        Err(_) => break true,
                    },
                    () = tokio::time::sleep_until(deadline) => {
                        expire_stale(forwarder);
                        stale_deadline = None;
                    }
                }
            } else {
                tokio::select! {
                    biased;
                    () = shutdown.cancelled() => break false,
                    changed = raw.changed() => match changed {
                        Ok(snapshot) => {
                            update_snapshot(
                                forwarder,
                                snapshot,
                                &mut stale_deadline,
                                config.max_staleness(),
                            );
                        }
                        Err(_) => break true,
                    }
                }
            }
        };
        if provider_disconnected {
            mark_disconnected(forwarder, &mut stale_deadline, config.max_staleness());
        }
        if let Err(error) = await_with_stale_deadline(
            close_generation(&handle, &state, &config),
            &mut stale_deadline,
            forwarder,
        )
        .await
        {
            terminal_state = SlotState::Failed(error);
            state.send_replace(terminal_state.clone());
            break;
        }
        if shutdown.is_cancelled() {
            break;
        }
        if !await_with_stale_deadline(
            reconnect_delay(&shutdown, &config, &mut reconnect_attempt),
            &mut stale_deadline,
            forwarder,
        )
        .await
        {
            break;
        }
        set_retrying_state(forwarder);
    }
    state.send_replace(terminal_state);
    if !shutdown.is_cancelled() {
        endpoint_breakers.remove_discovery(&key.selector);
        retained_publisher
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .take();
    }
    drop(publisher);
    record_directory_state(&metrics, &key.selector, DirectoryMetricState::Closed);
    completion.send_replace(true);
}

fn update_snapshot(
    forwarder: DirectoryForwarder<'_>,
    snapshot: fusen_register::directory::DirectorySnapshot,
    stale_deadline: &mut Option<tokio::time::Instant>,
    max_staleness: Duration,
) {
    match snapshot.state() {
        DirectoryState::Ready => {
            forwarder
                .endpoint_breakers
                .replace_discovery(forwarder.selector, snapshot.instances());
            let _ = forwarder
                .publisher
                .publish_snapshot(DirectoryState::Ready, snapshot.instances().to_vec());
            *stale_deadline = None;
            forwarder.state.send_replace(SlotState::Ready);
            record_directory_state(
                forwarder.metrics,
                forwarder.selector,
                DirectoryMetricState::Ready,
            );
        }
        DirectoryState::Stale
            if matches!(
                forwarder.directory.snapshot().state(),
                DirectoryState::Ready | DirectoryState::Stale
            ) =>
        {
            forwarder
                .endpoint_breakers
                .replace_discovery(forwarder.selector, snapshot.instances());
            let _ = forwarder
                .publisher
                .publish_snapshot(DirectoryState::Stale, snapshot.instances().to_vec());
            if stale_deadline.is_none() {
                *stale_deadline = Some(tokio::time::Instant::now() + max_staleness);
            }
            forwarder.state.send_replace(SlotState::Stale);
            record_directory_state(
                forwarder.metrics,
                forwarder.selector,
                DirectoryMetricState::Stale,
            );
        }
        DirectoryState::Stale | DirectoryState::Unavailable => {
            forwarder
                .endpoint_breakers
                .replace_discovery(forwarder.selector, snapshot.instances());
            let _ = forwarder
                .publisher
                .publish_snapshot(DirectoryState::Unavailable, snapshot.instances().to_vec());
            *stale_deadline = None;
            forwarder.state.send_replace(SlotState::Unavailable);
            record_directory_state(
                forwarder.metrics,
                forwarder.selector,
                DirectoryMetricState::Unavailable,
            );
        }
        DirectoryState::Closed => mark_disconnected(forwarder, stale_deadline, max_staleness),
        DirectoryState::Initializing => {}
        _ => {}
    }
}

fn mark_disconnected(
    forwarder: DirectoryForwarder<'_>,
    stale_deadline: &mut Option<tokio::time::Instant>,
    max_staleness: Duration,
) {
    match forwarder.directory.snapshot().state() {
        DirectoryState::Ready => {
            let _ = forwarder.publisher.publish_state(DirectoryState::Stale);
            if stale_deadline.is_none() {
                *stale_deadline = Some(tokio::time::Instant::now() + max_staleness);
            }
            forwarder.state.send_replace(SlotState::Stale);
            record_directory_state(
                forwarder.metrics,
                forwarder.selector,
                DirectoryMetricState::Stale,
            );
        }
        DirectoryState::Stale => {
            if stale_deadline.is_none() {
                *stale_deadline = Some(tokio::time::Instant::now() + max_staleness);
            }
            forwarder.state.send_replace(SlotState::Stale);
        }
        DirectoryState::Initializing => {
            let _ = forwarder
                .publisher
                .publish_state(DirectoryState::Unavailable);
            forwarder.state.send_replace(SlotState::Unavailable);
            record_directory_state(
                forwarder.metrics,
                forwarder.selector,
                DirectoryMetricState::Unavailable,
            );
        }
        DirectoryState::Unavailable | DirectoryState::Closed => {}
        _ => {}
    }
}

fn expire_stale(forwarder: DirectoryForwarder<'_>) {
    if forwarder.directory.snapshot().state() != DirectoryState::Stale {
        return;
    }
    let _ = forwarder
        .publisher
        .publish_state(DirectoryState::Unavailable);
    if matches!(
        forwarder.state.borrow().clone(),
        SlotState::Ready | SlotState::Stale
    ) {
        forwarder.state.send_replace(SlotState::Unavailable);
    }
    record_directory_state(
        forwarder.metrics,
        forwarder.selector,
        DirectoryMetricState::Unavailable,
    );
}

fn set_provider_failure(forwarder: DirectoryForwarder<'_>, error: RegistryError) {
    if forwarder.directory.snapshot().state() == DirectoryState::Stale {
        forwarder.state.send_replace(SlotState::Stale);
    } else {
        if forwarder.directory.snapshot().state() == DirectoryState::Initializing {
            let _ = forwarder
                .publisher
                .publish_state(DirectoryState::Unavailable);
        }
        forwarder.state.send_replace(SlotState::Failed(error));
        record_directory_state(
            forwarder.metrics,
            forwarder.selector,
            DirectoryMetricState::Unavailable,
        );
    }
}

fn set_retrying_state(forwarder: DirectoryForwarder<'_>) {
    match forwarder.directory.snapshot().state() {
        DirectoryState::Initializing => {
            forwarder.state.send_replace(SlotState::Initializing);
            record_directory_state(
                forwarder.metrics,
                forwarder.selector,
                DirectoryMetricState::Initializing,
            );
        }
        DirectoryState::Ready => {
            forwarder.state.send_replace(SlotState::Ready);
        }
        DirectoryState::Stale => {
            forwarder.state.send_replace(SlotState::Stale);
        }
        DirectoryState::Unavailable => {
            forwarder.state.send_replace(SlotState::Unavailable);
        }
        DirectoryState::Closed => {
            forwarder.state.send_replace(SlotState::Closed);
        }
        _ => {}
    }
}

async fn await_with_stale_deadline<F>(
    future: F,
    stale_deadline: &mut Option<tokio::time::Instant>,
    forwarder: DirectoryForwarder<'_>,
) -> F::Output
where
    F: Future,
{
    tokio::pin!(future);
    loop {
        let Some(deadline) = *stale_deadline else {
            return future.await;
        };
        tokio::select! {
            biased;
            output = &mut future => return output,
            () = tokio::time::sleep_until(deadline) => {
                expire_stale(forwarder);
                *stale_deadline = None;
            }
        }
    }
}

fn record_directory_state(
    metrics: &SafeMetrics,
    selector: &ServiceSelector,
    state: DirectoryMetricState,
) {
    metrics.record(&MetricEvent::DirectoryStateChanged(
        DirectoryStateChangedEvent::new(selector.service_id(), state),
    ));
}

async fn close_generation(
    handle: &SubscriptionHandle,
    state: &watch::Sender<SlotState>,
    config: &DiscoveryConfig,
) -> Result<(), RegistryError> {
    handle.request_close();
    match tokio::time::timeout(config.close_timeout(), handle.close()).await {
        Ok(result) => result,
        Err(_) => {
            state.send_replace(SlotState::Quarantined);
            handle.close().await
        }
    }
}

async fn reconnect_delay(
    shutdown: &CancellationToken,
    config: &DiscoveryConfig,
    attempt: &mut u8,
) -> bool {
    *attempt = attempt.saturating_add(1);
    let delay = full_jitter_backoff(
        config.reconnect_base(),
        config.reconnect_cap(),
        *attempt,
        &mut rand::rng(),
    );
    tokio::select! {
        biased;
        () = shutdown.cancelled() => false,
        () = tokio::time::sleep(delay) => true,
    }
}

fn closed() -> ClientError {
    ClientError::from_message(
        ClientErrorKind::Closed,
        "client runtime is draining or closed",
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::resilience::breaker::{BreakerConfig, DEFAULT_ENDPOINT_IDLE_EVICTION};
    use fusen_contract::{EndpointCapabilities, InstanceId, ServiceInstance, ServiceWeight};
    use fusen_register::{
        RegistrationHandle, RegistrationRequest, SubscriptionRequest,
        error::{RegistryErrorKind, RegistryOperation},
        provider,
    };
    use std::sync::atomic::AtomicUsize;
    use tokio::sync::{Notify, oneshot};

    struct ControlledRegistry {
        publisher: Arc<Mutex<Option<DirectoryPublisher>>>,
        activation_release: Mutex<Option<oneshot::Receiver<()>>>,
        close_release: Mutex<Option<oneshot::Receiver<()>>>,
        activation_started: Arc<Notify>,
        close_started: Arc<Notify>,
        close_count: Arc<AtomicUsize>,
        close_error: bool,
    }

    impl ControlledRegistry {
        fn new(
            activation_release: Option<oneshot::Receiver<()>>,
            close_release: Option<oneshot::Receiver<()>>,
            close_error: bool,
        ) -> Self {
            Self {
                publisher: Arc::new(Mutex::new(None)),
                activation_release: Mutex::new(activation_release),
                close_release: Mutex::new(close_release),
                activation_started: Arc::new(Notify::new()),
                close_started: Arc::new(Notify::new()),
                close_count: Arc::new(AtomicUsize::new(0)),
                close_error,
            }
        }

        fn publish_state(&self, state: DirectoryState) {
            self.publisher
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .as_ref()
                .expect("subscription publisher is installed")
                .publish_state(state)
                .unwrap();
        }

        fn disconnect(&self) {
            self.publisher
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .take();
        }

        fn close_count(&self) -> usize {
            self.close_count.load(Ordering::SeqCst)
        }
    }

    impl Registry for ControlledRegistry {
        fn prepare_registration(
            &self,
            _request: RegistrationRequest,
        ) -> Result<RegistrationHandle, RegistryError> {
            Err(RegistryError::message(
                RegistryOperation::PrepareRegistration,
                RegistryErrorKind::InvalidResource,
                "test registry does not publish registrations",
            ))
        }

        fn prepare_subscription(
            &self,
            _request: SubscriptionRequest,
        ) -> Result<SubscriptionHandle, RegistryError> {
            let (publisher, directory) = directory();
            *self
                .publisher
                .lock()
                .unwrap_or_else(|error| error.into_inner()) = Some(publisher.clone());
            let activation_release = self
                .activation_release
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .take();
            let close_release = self
                .close_release
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .take();
            let activation_started = self.activation_started.clone();
            let close_started = self.close_started.clone();
            let close_count = self.close_count.clone();
            let provider_publisher = self.publisher.clone();
            let close_error = self.close_error;
            Ok(provider::subscription(
                directory,
                async move {
                    activation_started.notify_one();
                    if let Some(release) = activation_release {
                        let _ = release.await;
                    }
                    publisher.publish_ready(vec![test_instance()])?;
                    Ok(())
                },
                move || async move {
                    close_count.fetch_add(1, Ordering::SeqCst);
                    close_started.notify_one();
                    if let Some(release) = close_release {
                        let _ = release.await;
                    }
                    provider_publisher
                        .lock()
                        .unwrap_or_else(|error| error.into_inner())
                        .take();
                    if close_error {
                        Err(RegistryError::message(
                            RegistryOperation::CloseSubscription,
                            RegistryErrorKind::Unavailable,
                            "expected close failure",
                        ))
                    } else {
                        Ok(())
                    }
                },
            ))
        }
    }

    fn test_instance() -> ServiceInstance {
        ServiceInstance::new(
            InstanceId::new("instance-1").unwrap(),
            "http://127.0.0.1:8080".parse().unwrap(),
            EndpointCapabilities::default(),
            ServiceWeight::default(),
        )
    }

    fn selector() -> ServiceSelector {
        ServiceSelector::new("subscription-test", None, None).unwrap()
    }

    fn named_selector(name: &str) -> ServiceSelector {
        ServiceSelector::new(name, None, None).unwrap()
    }

    fn discovery_config() -> DiscoveryConfig {
        DiscoveryConfig::builder()
            .initial_timeout(Duration::from_secs(30))
            .operation_timeout(Duration::from_secs(5))
            .close_timeout(Duration::from_secs(3))
            .max_staleness(Duration::from_secs(10))
            .reconnect_base(Duration::from_secs(1))
            .reconnect_cap(Duration::from_secs(1))
            .build()
            .unwrap()
    }

    fn endpoint_breakers() -> EndpointBreakers {
        EndpointBreakers::new(
            BreakerConfig::new(
                Duration::from_secs(10),
                10,
                20,
                0.5,
                Duration::from_secs(10),
                Duration::from_secs(120),
                1,
                2,
            ),
            10_000,
            DEFAULT_ENDPOINT_IDLE_EVICTION,
        )
    }

    fn current_slot_state(manager: &SubscriptionManager) -> SlotState {
        manager
            .slots
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .values()
            .next()
            .expect("one test subscription exists")
            .state
            .borrow()
            .clone()
    }

    async fn wait_for_directory_state(
        directory: &mut Directory,
        expected: DirectoryState,
    ) -> fusen_register::directory::DirectorySnapshot {
        loop {
            let snapshot = directory.changed().await.unwrap();
            if snapshot.state() == expected {
                return snapshot;
            }
        }
    }

    #[tokio::test(start_paused = true)]
    async fn stale_snapshot_expires_to_unavailable_and_shutdown_closes_once() {
        let registry = Arc::new(ControlledRegistry::new(None, None, false));
        let manager = SubscriptionManager::new(
            registry.clone(),
            discovery_config(),
            SafeMetrics::new(None),
            endpoint_breakers(),
        );
        let mut discovered = manager.acquire(selector()).await.unwrap();
        assert_eq!(discovered.snapshot().state(), DirectoryState::Ready);
        assert_eq!(discovered.snapshot().instances().len(), 1);

        registry.publish_state(DirectoryState::Stale);
        let stale = wait_for_directory_state(&mut discovered, DirectoryState::Stale).await;
        assert_eq!(stale.state(), DirectoryState::Stale);
        assert_eq!(stale.instances().len(), 1);

        tokio::time::advance(Duration::from_secs(10)).await;
        tokio::task::yield_now().await;
        let unavailable =
            wait_for_directory_state(&mut discovered, DirectoryState::Unavailable).await;
        assert_eq!(unavailable.state(), DirectoryState::Unavailable);
        assert_eq!(unavailable.instances().len(), 1);

        manager.begin_shutdown();
        manager.closed().await.unwrap();
        assert_eq!(registry.close_count(), 1);
        assert_eq!(discovered.snapshot().state(), DirectoryState::Unavailable);
        manager.finish_shutdown();
        assert_eq!(discovered.snapshot().state(), DirectoryState::Closed);
    }

    #[tokio::test]
    async fn provider_close_keeps_ready_directory_until_logical_drain_finishes() {
        let registry = Arc::new(ControlledRegistry::new(None, None, false));
        let manager = SubscriptionManager::new(
            registry.clone(),
            discovery_config(),
            SafeMetrics::new(None),
            endpoint_breakers(),
        );
        let discovered = manager.acquire(selector()).await.unwrap();

        manager.begin_shutdown();
        manager.closed().await.unwrap();
        assert_eq!(registry.close_count(), 1);
        assert_eq!(discovered.snapshot().state(), DirectoryState::Ready);

        manager.finish_shutdown();
        assert_eq!(discovered.snapshot().state(), DirectoryState::Closed);
    }

    #[tokio::test(start_paused = true)]
    async fn provider_disconnect_uses_last_good_until_one_deadline_then_recovers() {
        let (close_release, close_waiter) = oneshot::channel();
        let registry = Arc::new(ControlledRegistry::new(None, Some(close_waiter), false));
        let manager = SubscriptionManager::new(
            registry.clone(),
            discovery_config(),
            SafeMetrics::new(None),
            endpoint_breakers(),
        );
        let mut discovered = manager.acquire(selector()).await.unwrap();
        let ready_revision = discovered.snapshot().revision();

        registry.disconnect();
        let stale = wait_for_directory_state(&mut discovered, DirectoryState::Stale).await;
        registry.close_started.notified().await;
        assert!(stale.revision() > ready_revision);
        assert_eq!(stale.instances().len(), 1);
        assert!(manager.acquire(selector()).await.is_ok());

        tokio::time::advance(Duration::from_secs(10)).await;
        let unavailable =
            wait_for_directory_state(&mut discovered, DirectoryState::Unavailable).await;
        assert!(unavailable.revision() > stale.revision());
        assert_eq!(unavailable.instances().len(), 1);
        let error = manager.acquire(selector()).await.unwrap_err();
        assert_eq!(error.kind(), ClientErrorKind::Discovery);

        close_release.send(()).unwrap();
        tokio::time::advance(Duration::from_secs(1)).await;
        let recovered = wait_for_directory_state(&mut discovered, DirectoryState::Ready).await;
        assert!(recovered.revision() > unavailable.revision());
        assert_eq!(registry.close_count(), 1);

        manager.begin_shutdown();
        manager.closed().await.unwrap();
        manager.finish_shutdown();
        assert_eq!(registry.close_count(), 2);
    }

    #[tokio::test]
    async fn shutdown_waits_for_every_slot_before_returning_first_error() {
        let registry = Arc::new(ControlledRegistry::new(None, None, false));
        let manager = SubscriptionManager::new(
            registry,
            discovery_config(),
            SafeMetrics::new(None),
            endpoint_breakers(),
        );
        let (failed_publisher, failed_directory) = directory();
        let (pending_publisher, pending_directory) = directory();
        let expected = RegistryError::message(
            RegistryOperation::CloseSubscription,
            RegistryErrorKind::Unavailable,
            "first deterministic close failure",
        );
        let (_, failed_state) = watch::channel(SlotState::Failed(expected.clone()));
        let (_, pending_state) = watch::channel(SlotState::Closed);
        let (_, failed_completion) = watch::channel(true);
        let (pending_completion_sender, pending_completion) = watch::channel(false);
        {
            let mut slots = manager
                .slots
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            slots.insert(
                SubscriptionKey {
                    selector: named_selector("a-failed"),
                },
                Arc::new(Slot {
                    directory: failed_directory,
                    retained_publisher: Arc::new(Mutex::new(Some(failed_publisher))),
                    state: failed_state,
                    completion: failed_completion,
                }),
            );
            slots.insert(
                SubscriptionKey {
                    selector: named_selector("b-pending"),
                },
                Arc::new(Slot {
                    directory: pending_directory,
                    retained_publisher: Arc::new(Mutex::new(Some(pending_publisher))),
                    state: pending_state,
                    completion: pending_completion,
                }),
            );
        }

        let closing = tokio::spawn({
            let manager = manager.clone();
            async move { manager.closed().await }
        });
        tokio::task::yield_now().await;
        assert!(!closing.is_finished());

        pending_completion_sender.send_replace(true);
        let error = closing.await.unwrap().unwrap_err();
        assert_eq!(error.to_string(), expected.to_string());
    }

    #[tokio::test(start_paused = true)]
    async fn activation_timeout_quarantines_until_late_activation_is_compensated() {
        let (activation_release, activation_waiter) = oneshot::channel();
        let registry = Arc::new(ControlledRegistry::new(
            Some(activation_waiter),
            None,
            false,
        ));
        let manager = SubscriptionManager::new(
            registry.clone(),
            discovery_config(),
            SafeMetrics::new(None),
            endpoint_breakers(),
        );
        let acquiring = tokio::spawn({
            let manager = manager.clone();
            async move { manager.acquire(selector()).await }
        });
        registry.activation_started.notified().await;

        tokio::time::advance(Duration::from_secs(5)).await;
        tokio::task::yield_now().await;
        let error = acquiring.await.unwrap().unwrap_err();
        assert_eq!(error.kind(), ClientErrorKind::Discovery);
        assert!(matches!(
            current_slot_state(&manager),
            SlotState::Quarantined
        ));

        manager.begin_shutdown();
        activation_release.send(()).unwrap();
        registry.close_started.notified().await;
        manager.closed().await.unwrap();
        manager.finish_shutdown();
        assert_eq!(registry.close_count(), 1);
        assert!(matches!(current_slot_state(&manager), SlotState::Closed));
    }

    #[tokio::test(start_paused = true)]
    async fn close_timeout_quarantines_without_starting_duplicate_cleanup() {
        let (close_release, close_waiter) = oneshot::channel();
        let registry = Arc::new(ControlledRegistry::new(None, Some(close_waiter), false));
        let manager = SubscriptionManager::new(
            registry.clone(),
            discovery_config(),
            SafeMetrics::new(None),
            endpoint_breakers(),
        );
        manager.acquire(selector()).await.unwrap();
        manager.begin_shutdown();
        let closing = tokio::spawn({
            let manager = manager.clone();
            async move { manager.closed().await }
        });
        registry.close_started.notified().await;

        tokio::time::advance(Duration::from_secs(3)).await;
        tokio::task::yield_now().await;
        assert!(matches!(
            current_slot_state(&manager),
            SlotState::Quarantined
        ));
        assert_eq!(registry.close_count(), 1);

        close_release.send(()).unwrap();
        closing.await.unwrap().unwrap();
        manager.finish_shutdown();
        assert_eq!(registry.close_count(), 1);
        assert!(matches!(current_slot_state(&manager), SlotState::Closed));
    }

    #[tokio::test(start_paused = true)]
    async fn provider_close_error_is_preserved_as_shutdown_failure() {
        let registry = Arc::new(ControlledRegistry::new(None, None, true));
        let manager = SubscriptionManager::new(
            registry.clone(),
            discovery_config(),
            SafeMetrics::new(None),
            endpoint_breakers(),
        );
        manager.acquire(selector()).await.unwrap();
        manager.begin_shutdown();

        let error = manager.closed().await.unwrap_err();
        manager.finish_shutdown();
        assert_eq!(error.operation(), RegistryOperation::CloseSubscription);
        assert_eq!(error.kind(), RegistryErrorKind::Unavailable);
        assert_eq!(registry.close_count(), 1);
        assert!(matches!(current_slot_state(&manager), SlotState::Failed(_)));
    }
}
