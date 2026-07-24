use crate::error::FusenError;
use fusen_contract::{ServiceSelector, WireProtocol};
use fusen_register::{Register, ServiceSubscription, directory::Directory, error::RegisterError};
use std::{
    collections::HashMap,
    sync::{Arc, Weak},
    time::Duration,
};
use tokio::{
    runtime::Handle,
    sync::{Mutex, Notify, futures::OwnedNotified, watch},
};

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) struct SubscriptionKey {
    selector: ServiceSelector,
    protocol: WireProtocol,
}

impl SubscriptionKey {
    pub(crate) fn new(selector: ServiceSelector, protocol: WireProtocol) -> Self {
        Self { selector, protocol }
    }
}

#[derive(Clone)]
enum CreationFailure {
    Timeout,
    Registry(RegisterError),
}

#[derive(Clone)]
enum CreationOutcome {
    Pending,
    Ready,
    Failed(CreationFailure),
}

enum Entry {
    Creating {
        generation: u64,
        outcome: watch::Sender<CreationOutcome>,
    },
    Active {
        generation: u64,
        subscription: ServiceSubscription,
        leases: usize,
    },
    Closing {
        generation: u64,
        subscription: ServiceSubscription,
        changed: Arc<Notify>,
    },
}

struct State {
    closed: bool,
    next_generation: u64,
    entries: HashMap<SubscriptionKey, Entry>,
    close_failure: Option<RegisterError>,
}

pub(crate) struct SubscriptionManager {
    state: Mutex<State>,
    close_timeout: Duration,
}

impl SubscriptionManager {
    pub(crate) fn new(close_timeout: Duration) -> Arc<Self> {
        Arc::new(Self {
            state: Mutex::new(State {
                closed: false,
                next_generation: 0,
                entries: HashMap::new(),
                close_failure: None,
            }),
            close_timeout,
        })
    }

    pub(crate) async fn acquire(
        self: &Arc<Self>,
        key: SubscriptionKey,
        registry: Arc<dyn Register>,
        discovery_timeout: Duration,
    ) -> Result<SubscriptionLease, FusenError> {
        loop {
            enum Action {
                Start(u64, watch::Receiver<CreationOutcome>),
                WaitCreation(watch::Receiver<CreationOutcome>),
                WaitClosing(OwnedNotified),
                Ready(Directory),
            }

            let action = {
                let mut state = self.state.lock().await;
                if state.closed {
                    return Err(closed_error());
                }
                match state.entries.get_mut(&key) {
                    Some(Entry::Creating { outcome, .. }) => {
                        Action::WaitCreation(outcome.subscribe())
                    }
                    Some(Entry::Closing { changed, .. }) => {
                        Action::WaitClosing(changed.clone().notified_owned())
                    }
                    Some(Entry::Active {
                        subscription,
                        leases,
                        ..
                    }) => {
                        *leases += 1;
                        Action::Ready(subscription.directory().clone())
                    }
                    None => {
                        let generation = state.next_generation;
                        state.next_generation = state.next_generation.wrapping_add(1);
                        let (outcome, receiver) = watch::channel(CreationOutcome::Pending);
                        state.entries.insert(
                            key.clone(),
                            Entry::Creating {
                                generation,
                                outcome,
                            },
                        );
                        Action::Start(generation, receiver)
                    }
                }
            };

            match action {
                Action::Start(generation, receiver) => {
                    let manager = self.clone();
                    let create_key = key.clone();
                    let registry = registry.clone();
                    tokio::spawn(async move {
                        let subscribe_key = create_key.clone();
                        let subscription = tokio::spawn(async move {
                            match tokio::time::timeout(
                                discovery_timeout,
                                registry.subscribe(
                                    subscribe_key.selector.clone(),
                                    subscribe_key.protocol,
                                ),
                            )
                            .await
                            {
                                Ok(Ok(subscription)) => Ok(subscription),
                                Ok(Err(error)) => Err(CreationFailure::Registry(error)),
                                Err(_) => Err(CreationFailure::Timeout),
                            }
                        });
                        let result = match subscription.await {
                            Ok(result) => result,
                            Err(error) => Err(CreationFailure::Registry(RegisterError::provider(
                                std::io::Error::other(format!(
                                    "subscription creation task failed: {error}"
                                )),
                            ))),
                        };
                        manager
                            .complete_creation(create_key, generation, result)
                            .await;
                    });
                    wait_for_creation(receiver).await?;
                }
                Action::WaitCreation(receiver) => wait_for_creation(receiver).await?,
                Action::WaitClosing(changed) => changed.await,
                Action::Ready(directory) => {
                    return Ok(SubscriptionLease {
                        directory,
                        manager: Arc::downgrade(self),
                        key: Some(key),
                        runtime: Handle::current(),
                    });
                }
            }
        }
    }

    async fn complete_creation(
        self: &Arc<Self>,
        key: SubscriptionKey,
        generation: u64,
        result: Result<ServiceSubscription, CreationFailure>,
    ) {
        let mut state = self.state.lock().await;
        let Some(Entry::Creating {
            generation: current,
            outcome,
        }) = state.entries.get(&key)
        else {
            if let Ok(subscription) = result {
                drop(state);
                self.close_untracked(subscription).await;
            }
            return;
        };
        if *current != generation {
            if let Ok(subscription) = result {
                drop(state);
                self.close_untracked(subscription).await;
            }
            return;
        }
        let outcome = outcome.clone();
        let completed = match result {
            Ok(subscription) if outcome.receiver_count() == 0 => {
                state.entries.remove(&key);
                drop(state);
                self.close_untracked(subscription).await;
                return;
            }
            Ok(subscription) => {
                state.entries.insert(
                    key,
                    Entry::Active {
                        generation,
                        subscription,
                        leases: 0,
                    },
                );
                CreationOutcome::Ready
            }
            Err(failure) => {
                state.entries.remove(&key);
                CreationOutcome::Failed(failure)
            }
        };
        drop(state);
        outcome.send_replace(completed);
    }

    async fn close_untracked(&self, subscription: ServiceSubscription) {
        let _ = tokio::time::timeout(self.close_timeout, subscription.close()).await;
    }

    async fn release(self: Arc<Self>, key: SubscriptionKey) {
        let closing = {
            let mut state = self.state.lock().await;
            let Some(Entry::Active {
                generation,
                subscription,
                leases,
            }) = state.entries.get_mut(&key)
            else {
                return;
            };
            if *leases > 1 {
                *leases -= 1;
                return;
            }
            if *leases == 0 {
                return;
            }
            let generation = *generation;
            let subscription = subscription.clone();
            let changed = Arc::new(Notify::new());
            state.entries.insert(
                key.clone(),
                Entry::Closing {
                    generation,
                    subscription: subscription.clone(),
                    changed: changed.clone(),
                },
            );
            (generation, subscription, changed)
        };

        let (generation, subscription, changed) = closing;
        let first = tokio::time::timeout(self.close_timeout, subscription.close()).await;
        let result = match first {
            Ok(result) => result,
            Err(_) => subscription.close().await,
        };
        let mut state = self.state.lock().await;
        if matches!(
            state.entries.get(&key),
            Some(Entry::Closing { generation: current, .. }) if *current == generation
        ) {
            state.entries.remove(&key);
            if let Err(error) = result
                && state.close_failure.is_none()
            {
                state.close_failure = Some(error);
            }
        }
        drop(state);
        changed.notify_waiters();
    }

    pub(crate) async fn shutdown(self: &Arc<Self>) -> Result<(), ManagerShutdownError> {
        loop {
            let creating = {
                let mut state = self.state.lock().await;
                state.closed = true;
                state
                    .entries
                    .values()
                    .filter_map(|entry| match entry {
                        Entry::Creating { outcome, .. } => Some(outcome.subscribe()),
                        _ => None,
                    })
                    .collect::<Vec<_>>()
            };
            if creating.is_empty() {
                break;
            }
            for outcome in creating {
                let _ = wait_for_creation(outcome).await;
            }
        }

        let pending = {
            let state = self.state.lock().await;
            state
                .entries
                .iter()
                .filter_map(|(key, entry)| match entry {
                    Entry::Active {
                        generation,
                        subscription,
                        ..
                    }
                    | Entry::Closing {
                        generation,
                        subscription,
                        ..
                    } => Some((key.clone(), *generation, subscription.clone())),
                    Entry::Creating { .. } => None,
                })
                .collect::<Vec<_>>()
        };

        let mut tasks = Vec::with_capacity(pending.len());
        for (key, generation, subscription) in pending {
            let timeout = self.close_timeout;
            tasks.push((
                key,
                generation,
                tokio::spawn(
                    async move { tokio::time::timeout(timeout, subscription.close()).await },
                ),
            ));
        }

        let mut timed_out = false;
        for (key, generation, task) in tasks {
            let outcome = match task.await {
                Ok(result) => result,
                Err(error) => Ok(Err(RegisterError::provider(std::io::Error::other(
                    format!("subscription cleanup task failed: {error}"),
                )))),
            };
            let mut state = self.state.lock().await;
            let current = matches!(
                state.entries.get(&key),
                Some(Entry::Active { generation: value, .. } | Entry::Closing { generation: value, .. })
                    if *value == generation
            );
            match outcome {
                Ok(Ok(())) if current => {
                    state.entries.remove(&key);
                }
                Ok(Err(error)) if current => {
                    state.entries.remove(&key);
                    if state.close_failure.is_none() {
                        state.close_failure = Some(error);
                    }
                }
                Err(_) if current => timed_out = true,
                Ok(Ok(())) | Ok(Err(_)) | Err(_) => {}
            }
        }

        let state = self.state.lock().await;
        if timed_out {
            Err(ManagerShutdownError::Timeout)
        } else if let Some(error) = &state.close_failure {
            Err(ManagerShutdownError::Terminal(error.clone()))
        } else {
            Ok(())
        }
    }
}

pub(crate) struct SubscriptionLease {
    directory: Directory,
    manager: Weak<SubscriptionManager>,
    key: Option<SubscriptionKey>,
    runtime: Handle,
}

impl SubscriptionLease {
    pub(crate) fn directory(&self) -> &Directory {
        &self.directory
    }
}

impl Drop for SubscriptionLease {
    fn drop(&mut self) {
        let Some(key) = self.key.take() else {
            return;
        };
        let Some(manager) = self.manager.upgrade() else {
            return;
        };
        self.runtime.spawn(manager.release(key));
    }
}

pub(crate) enum ManagerShutdownError {
    Timeout,
    Terminal(RegisterError),
}

fn creation_error(failure: CreationFailure) -> FusenError {
    match failure {
        CreationFailure::Timeout => {
            FusenError::Timeout("service discovery deadline exceeded".into())
        }
        CreationFailure::Registry(error) => {
            FusenError::internal("service subscription failed", error)
        }
    }
}

async fn wait_for_creation(
    mut outcome: watch::Receiver<CreationOutcome>,
) -> Result<(), FusenError> {
    loop {
        match outcome.borrow().clone() {
            CreationOutcome::Pending => {}
            CreationOutcome::Ready => return Ok(()),
            CreationOutcome::Failed(failure) => return Err(creation_error(failure)),
        }
        outcome.changed().await.map_err(|_| {
            FusenError::ServiceUnavailable("service subscription creation was cancelled".into())
        })?;
    }
}

fn closed_error() -> FusenError {
    FusenError::ServiceUnavailable("client runtime is shut down".into())
}
