#![warn(missing_docs)]
//! Cancellation-safe service registration and discovery contracts for fusen-rs.

use fusen_contract::{ServiceRegistration, ServiceSelector};
use futures_util::FutureExt;
use std::{
    future::Future,
    panic::{AssertUnwindSafe, catch_unwind},
    pin::Pin,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
};
use tokio::{runtime::Handle, sync::watch};

use crate::{
    directory::Directory,
    error::{RegistryError, RegistryErrorKind, RegistryOperation},
};

/// Latest-wins service directories and provider publication handles.
pub mod directory;
/// Classified registry failures.
pub mod error;
/// Owned, sendable future returned by registry lifecycle APIs.
pub type RegistryFuture<T> =
    Pin<Box<dyn Future<Output = Result<T, RegistryError>> + Send + 'static>>;

/// Parameters for preparing one provider registration.
#[derive(Clone, Debug)]
pub struct RegistrationRequest {
    registration: Arc<ServiceRegistration>,
}

impl RegistrationRequest {
    /// Creates a registration request.
    pub fn new(registration: Arc<ServiceRegistration>) -> Self {
        Self { registration }
    }

    /// Returns the immutable provider registration.
    pub fn registration(&self) -> &Arc<ServiceRegistration> {
        &self.registration
    }

    /// Consumes this request into the immutable provider registration.
    pub fn into_registration(self) -> Arc<ServiceRegistration> {
        self.registration
    }
}

/// Parameters for preparing one discovery subscription.
#[derive(Clone, Debug)]
pub struct SubscriptionRequest {
    selector: ServiceSelector,
}

impl SubscriptionRequest {
    /// Creates a subscription request.
    pub fn new(selector: ServiceSelector) -> Self {
        Self { selector }
    }

    /// Returns the service selector.
    pub const fn selector(&self) -> &ServiceSelector {
        &self.selector
    }

    /// Consumes this request into the service selector.
    pub fn into_selector(self) -> ServiceSelector {
        self.selector
    }
}

/// Pluggable provider that prepares registration and subscription ownership before activation.
///
/// Implementations must construct handles without starting remote side effects. The runtime stores
/// each handle before calling its `activate` method, so cancellation always has a cleanup owner.
pub trait Registry: Send + Sync + 'static {
    /// Prepares one service registration without publishing it yet.
    fn prepare_registration(
        &self,
        request: RegistrationRequest,
    ) -> Result<RegistrationHandle, RegistryError>;

    /// Prepares one service subscription without installing it yet.
    fn prepare_subscription(
        &self,
        request: SubscriptionRequest,
    ) -> Result<SubscriptionHandle, RegistryError>;
}

impl<T> Registry for Arc<T>
where
    T: Registry + ?Sized,
{
    fn prepare_registration(
        &self,
        request: RegistrationRequest,
    ) -> Result<RegistrationHandle, RegistryError> {
        (**self).prepare_registration(request)
    }

    fn prepare_subscription(
        &self,
        request: SubscriptionRequest,
    ) -> Result<SubscriptionHandle, RegistryError> {
        (**self).prepare_subscription(request)
    }
}

/// Safe constructors for provider-owned registry lifecycles.
pub mod provider {
    use super::*;

    /// Creates a registration handle from activation and cleanup operations.
    pub fn registration<A, C, CF>(activate: A, close: C) -> RegistrationHandle
    where
        A: Future<Output = Result<(), RegistryError>> + Send + 'static,
        C: FnOnce() -> CF + Send + 'static,
        CF: Future<Output = Result<(), RegistryError>> + Send + 'static,
    {
        super::prepare_registration(activate, close)
    }

    /// Creates a subscription handle and stable directory from provider operations.
    pub fn subscription<A, C, CF>(directory: Directory, activate: A, close: C) -> SubscriptionHandle
    where
        A: Future<Output = Result<(), RegistryError>> + Send + 'static,
        C: FnOnce() -> CF + Send + 'static,
        CF: Future<Output = Result<(), RegistryError>> + Send + 'static,
    {
        super::prepare_subscription(directory, activate, close)
    }
}

/// Creates a registration handle from provider-owned activation and cleanup operations.
///
/// Neither future is polled before the first call to [`RegistrationHandle::activate`]. Cleanup is
/// constructed at most once and only after activation has reached a terminal result.
fn prepare_registration<A, C, CF>(activate: A, close: C) -> RegistrationHandle
where
    A: Future<Output = Result<(), RegistryError>> + Send + 'static,
    C: FnOnce() -> CF + Send + 'static,
    CF: Future<Output = Result<(), RegistryError>> + Send + 'static,
{
    RegistrationHandle {
        lifecycle: Arc::new(Lifecycle::new(
            RegistryOperation::ActivateRegistration,
            RegistryOperation::CloseRegistration,
            Box::pin(activate),
            Box::new(move || Box::pin(close())),
        )),
    }
}

/// Creates a subscription handle from a provider directory, activation, and cleanup operations.
///
/// Neither future is polled before the first call to [`SubscriptionHandle::activate`]. Cleanup is
/// constructed at most once and only after activation has reached a terminal result.
fn prepare_subscription<A, C, CF>(directory: Directory, activate: A, close: C) -> SubscriptionHandle
where
    A: Future<Output = Result<(), RegistryError>> + Send + 'static,
    C: FnOnce() -> CF + Send + 'static,
    CF: Future<Output = Result<(), RegistryError>> + Send + 'static,
{
    SubscriptionHandle {
        lifecycle: Arc::new(Lifecycle::new(
            RegistryOperation::ActivateSubscription,
            RegistryOperation::CloseSubscription,
            Box::pin(activate),
            Box::new(move || Box::pin(close())),
        )),
        directory,
    }
}

/// Prepared ownership of one provider registration.
///
/// Clones share activation and cleanup terminal results. Dropping the last clone only requests
/// cleanup; provider work remains owned by the worker started during activation. Cancelling every
/// pending activation waiter also requests cleanup, so a late provider success is compensated.
#[derive(Clone)]
pub struct RegistrationHandle {
    lifecycle: Arc<Lifecycle>,
}

impl RegistrationHandle {
    /// Starts provider activation once and shares its terminal result with every caller.
    pub fn activate(&self) -> RegistryFuture<()> {
        let lifecycle = self.lifecycle.clone();
        let mut waiter = ActivationWaiter::new(lifecycle.clone());
        Box::pin(async move {
            let result = match lifecycle.ensure_started() {
                Ok(()) => lifecycle.wait_activation().await,
                Err(error) => Err(error),
            };
            waiter.complete();
            result
        })
    }

    /// Requests cleanup without waiting for provider completion.
    pub fn request_close(&self) {
        self.lifecycle.request_close();
    }

    /// Requests cleanup and shares its terminal result with every caller.
    pub fn close(&self) -> RegistryFuture<()> {
        let lifecycle = self.lifecycle.clone();
        Box::pin(async move {
            lifecycle.request_close();
            lifecycle.wait_close().await
        })
    }
}

impl std::fmt::Debug for RegistrationHandle {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("RegistrationHandle")
            .finish_non_exhaustive()
    }
}

/// Prepared ownership of one provider subscription and its stable directory.
///
/// Clones share activation and cleanup terminal results. Dropping the last clone only requests
/// cleanup; provider work remains owned by the worker started during activation. Cancelling every
/// pending activation waiter also requests cleanup, so a late provider success is compensated.
#[derive(Clone)]
pub struct SubscriptionHandle {
    lifecycle: Arc<Lifecycle>,
    directory: Directory,
}

impl SubscriptionHandle {
    /// Starts provider activation once and returns the shared directory after successful setup.
    pub fn activate(&self) -> RegistryFuture<Directory> {
        let lifecycle = self.lifecycle.clone();
        let directory = self.directory.clone();
        let mut waiter = ActivationWaiter::new(lifecycle.clone());
        Box::pin(async move {
            let result = match lifecycle.ensure_started() {
                Ok(()) => lifecycle.wait_activation().await.map(|()| directory),
                Err(error) => Err(error),
            };
            waiter.complete();
            result
        })
    }

    /// Requests cleanup without waiting for provider completion.
    pub fn request_close(&self) {
        self.lifecycle.request_close();
    }

    /// Requests cleanup and shares its terminal result with every caller.
    pub fn close(&self) -> RegistryFuture<()> {
        let lifecycle = self.lifecycle.clone();
        Box::pin(async move {
            lifecycle.request_close();
            lifecycle.wait_close().await
        })
    }
}

impl std::fmt::Debug for SubscriptionHandle {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("SubscriptionHandle")
            .field("directory", &self.directory)
            .finish_non_exhaustive()
    }
}

type CloseFactory = Box<dyn FnOnce() -> RegistryFuture<()> + Send + 'static>;
type SharedResult = Option<Result<(), RegistryError>>;

struct PreparedLifecycle {
    activate: RegistryFuture<()>,
    close: CloseFactory,
}

enum StartState {
    Prepared(Option<PreparedLifecycle>),
    Started,
    Finished,
}

struct Lifecycle {
    activation_operation: RegistryOperation,
    close_operation: RegistryOperation,
    start: Mutex<StartState>,
    activation_waiters: AtomicUsize,
    activation_observed: AtomicBool,
    close_requested: AtomicBool,
    close_request: watch::Sender<bool>,
    activation_result: watch::Sender<SharedResult>,
    close_result: watch::Sender<SharedResult>,
}

impl Lifecycle {
    fn new(
        activation_operation: RegistryOperation,
        close_operation: RegistryOperation,
        activate: RegistryFuture<()>,
        close: CloseFactory,
    ) -> Self {
        let (close_request, _) = watch::channel(false);
        let (activation_result, _) = watch::channel(None);
        let (close_result, _) = watch::channel(None);
        Self {
            activation_operation,
            close_operation,
            start: Mutex::new(StartState::Prepared(Some(PreparedLifecycle {
                activate,
                close,
            }))),
            activation_waiters: AtomicUsize::new(0),
            activation_observed: AtomicBool::new(false),
            close_requested: AtomicBool::new(false),
            close_request,
            activation_result,
            close_result,
        }
    }

    fn ensure_started(&self) -> Result<(), RegistryError> {
        if self.activation_result.borrow().is_some() {
            return Ok(());
        }
        let runtime = match Handle::try_current() {
            Ok(runtime) => runtime,
            Err(error) => {
                let error = RegistryError::new(
                    self.activation_operation,
                    RegistryErrorKind::Internal,
                    error,
                );
                self.finish_before_start(Err(error.clone()));
                return Err(error);
            }
        };
        let mut start = self.start.lock().unwrap_or_else(|error| error.into_inner());
        match &mut *start {
            StartState::Started => return Ok(()),
            StartState::Finished => return Ok(()),
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
        let worker = LifecycleWorker {
            activation_operation: self.activation_operation,
            close_operation: self.close_operation,
            activate: Some(prepared.activate),
            close: Some(prepared.close),
            close_request: self.close_request.subscribe(),
            activation_result: self.activation_result.clone(),
            close_result: self.close_result.clone(),
            activation_published: false,
            close_published: false,
        };
        runtime.spawn(worker.run());
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

    fn finish_before_start(&self, activation: Result<(), RegistryError>) {
        let mut start = self.start.lock().unwrap_or_else(|error| error.into_inner());
        let prepared = match &mut *start {
            StartState::Prepared(prepared) => prepared.take(),
            StartState::Started | StartState::Finished => return,
        };
        drop(prepared);
        *start = StartState::Finished;
        self.activation_result.send_replace(Some(activation));
        self.close_result.send_replace(Some(Ok(())));
    }

    fn publish_pre_activation_close(&self) {
        self.activation_result
            .send_replace(Some(Err(RegistryError::message(
                self.activation_operation,
                RegistryErrorKind::Cancelled,
                "registry handle closed before activation",
            ))));
        self.close_result.send_replace(Some(Ok(())));
    }

    async fn wait_activation(&self) -> Result<(), RegistryError> {
        wait_for_result(
            self.activation_result.subscribe(),
            self.activation_operation,
            "activation worker ended without a result",
        )
        .await
    }

    async fn wait_close(&self) -> Result<(), RegistryError> {
        wait_for_result(
            self.close_result.subscribe(),
            self.close_operation,
            "cleanup worker ended without a result",
        )
        .await
    }
}

struct ActivationWaiter {
    lifecycle: Arc<Lifecycle>,
    registered: bool,
    completed: bool,
}

impl ActivationWaiter {
    fn new(lifecycle: Arc<Lifecycle>) -> Self {
        lifecycle.activation_waiters.fetch_add(1, Ordering::AcqRel);
        Self {
            lifecycle,
            registered: true,
            completed: false,
        }
    }

    fn complete(&mut self) {
        self.completed = true;
        self.lifecycle
            .activation_observed
            .store(true, Ordering::Release);
        self.release();
    }

    fn release(&mut self) {
        if !self.registered {
            return;
        }
        self.registered = false;
        let previous = self
            .lifecycle
            .activation_waiters
            .fetch_sub(1, Ordering::AcqRel);
        debug_assert!(previous > 0);
        if previous == 1
            && !self.completed
            && !self.lifecycle.activation_observed.load(Ordering::Acquire)
        {
            self.lifecycle.request_close();
        }
    }
}

impl Drop for ActivationWaiter {
    fn drop(&mut self) {
        self.release();
    }
}

impl Drop for Lifecycle {
    fn drop(&mut self) {
        self.request_close();
    }
}

struct LifecycleWorker {
    activation_operation: RegistryOperation,
    close_operation: RegistryOperation,
    activate: Option<RegistryFuture<()>>,
    close: Option<CloseFactory>,
    close_request: watch::Receiver<bool>,
    activation_result: watch::Sender<SharedResult>,
    close_result: watch::Sender<SharedResult>,
    activation_published: bool,
    close_published: bool,
}

impl LifecycleWorker {
    async fn run(mut self) {
        let activate = self
            .activate
            .take()
            .expect("activation future is present until the worker starts");
        let activation = match AssertUnwindSafe(activate).catch_unwind().await {
            Ok(result) => result,
            Err(_) => Err(RegistryError::message(
                self.activation_operation,
                RegistryErrorKind::Internal,
                "registry provider activation panicked",
            )),
        };
        let activation = if *self.close_request.borrow() && activation.is_ok() {
            Err(RegistryError::message(
                self.activation_operation,
                RegistryErrorKind::Cancelled,
                "registry handle closed while activation was pending",
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
        let close = match catch_unwind(AssertUnwindSafe(close)) {
            Ok(close) => match AssertUnwindSafe(close).catch_unwind().await {
                Ok(result) => result,
                Err(_) => Err(RegistryError::message(
                    self.close_operation,
                    RegistryErrorKind::Internal,
                    "registry provider cleanup panicked",
                )),
            },
            Err(_) => Err(RegistryError::message(
                self.close_operation,
                RegistryErrorKind::Internal,
                "registry provider cleanup factory panicked",
            )),
        };
        self.close_result.send_replace(Some(close));
        self.close_published = true;
    }
}

impl Drop for LifecycleWorker {
    fn drop(&mut self) {
        if !self.activation_published {
            self.activation_result
                .send_replace(Some(Err(RegistryError::message(
                    self.activation_operation,
                    RegistryErrorKind::Internal,
                    "registry activation worker was aborted",
                ))));
        }
        if !self.close_published {
            self.close_result
                .send_replace(Some(Err(RegistryError::message(
                    self.close_operation,
                    RegistryErrorKind::CleanupAborted,
                    "registry cleanup worker was aborted",
                ))));
        }
    }
}

async fn wait_for_result(
    mut result: watch::Receiver<SharedResult>,
    operation: RegistryOperation,
    ended_message: &'static str,
) -> Result<(), RegistryError> {
    loop {
        if let Some(result) = result.borrow().clone() {
            return result;
        }
        result.changed().await.map_err(|_| {
            RegistryError::message(operation, RegistryErrorKind::Internal, ended_message)
        })?;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::directory::directory;
    use std::{
        sync::atomic::{AtomicUsize, Ordering},
        task::{Context, Poll, Waker},
    };
    use tokio::sync::{Notify, oneshot};

    fn close_error(message: &str) -> RegistryError {
        RegistryError::message(
            RegistryOperation::CloseRegistration,
            RegistryErrorKind::Internal,
            message,
        )
    }

    #[tokio::test]
    async fn prepared_handle_has_no_side_effect_before_activation() {
        let activations = Arc::new(AtomicUsize::new(0));
        let cleanups = Arc::new(AtomicUsize::new(0));
        let handle = prepare_registration(
            {
                let activations = activations.clone();
                async move {
                    activations.fetch_add(1, Ordering::SeqCst);
                    Ok(())
                }
            },
            {
                let cleanups = cleanups.clone();
                move || async move {
                    cleanups.fetch_add(1, Ordering::SeqCst);
                    Ok(())
                }
            },
        );

        tokio::task::yield_now().await;
        assert_eq!(activations.load(Ordering::SeqCst), 0);
        handle.close().await.unwrap();
        assert_eq!(activations.load(Ordering::SeqCst), 0);
        assert_eq!(cleanups.load(Ordering::SeqCst), 0);
        assert_eq!(
            handle.activate().await.unwrap_err().kind(),
            RegistryErrorKind::Cancelled
        );
    }

    #[tokio::test]
    async fn cancelling_last_activation_waiter_requests_late_success_cleanup() {
        let started = Arc::new(Notify::new());
        let (release_sender, release_receiver) = oneshot::channel();
        let cleanups = Arc::new(AtomicUsize::new(0));
        let cleanup_completed = Arc::new(Notify::new());
        let handle = prepare_registration(
            {
                let started = started.clone();
                async move {
                    started.notify_one();
                    let _ = release_receiver.await;
                    Ok(())
                }
            },
            {
                let cleanups = cleanups.clone();
                let cleanup_completed = cleanup_completed.clone();
                move || async move {
                    cleanups.fetch_add(1, Ordering::SeqCst);
                    cleanup_completed.notify_one();
                    Ok(())
                }
            },
        );
        let waiter = tokio::spawn({
            let handle = handle.clone();
            async move { handle.activate().await }
        });
        started.notified().await;
        waiter.abort();
        assert!(waiter.await.unwrap_err().is_cancelled());

        release_sender.send(()).unwrap();
        cleanup_completed.notified().await;
        assert_eq!(cleanups.load(Ordering::SeqCst), 1);
        handle.close().await.unwrap();
        assert_eq!(
            handle.activate().await.unwrap_err().kind(),
            RegistryErrorKind::Cancelled
        );
    }

    #[tokio::test]
    async fn cancelling_one_of_two_activation_waiters_keeps_the_shared_activation_alive() {
        let started = Arc::new(Notify::new());
        let (release_sender, release_receiver) = oneshot::channel();
        let cleanups = Arc::new(AtomicUsize::new(0));
        let handle = prepare_registration(
            {
                let started = started.clone();
                async move {
                    started.notify_one();
                    let _ = release_receiver.await;
                    Ok(())
                }
            },
            {
                let cleanups = cleanups.clone();
                move || async move {
                    cleanups.fetch_add(1, Ordering::SeqCst);
                    Ok(())
                }
            },
        );
        let cancelled_waiter = handle.activate();
        let surviving_waiter = handle.activate();
        let cancelled = tokio::spawn(cancelled_waiter);
        let surviving = tokio::spawn(surviving_waiter);
        started.notified().await;

        cancelled.abort();
        assert!(cancelled.await.unwrap_err().is_cancelled());
        release_sender.send(()).unwrap();
        surviving.await.unwrap().unwrap();
        assert_eq!(cleanups.load(Ordering::SeqCst), 0);

        handle.close().await.unwrap();
        assert_eq!(cleanups.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn concurrent_activation_and_close_share_one_provider_operation() {
        let activations = Arc::new(AtomicUsize::new(0));
        let cleanups = Arc::new(AtomicUsize::new(0));
        let close_started = Arc::new(Notify::new());
        let close_release = Arc::new(Notify::new());
        let handle = prepare_registration(
            {
                let activations = activations.clone();
                async move {
                    activations.fetch_add(1, Ordering::SeqCst);
                    Ok(())
                }
            },
            {
                let cleanups = cleanups.clone();
                let close_started = close_started.clone();
                let close_release = close_release.clone();
                move || async move {
                    cleanups.fetch_add(1, Ordering::SeqCst);
                    close_started.notify_one();
                    close_release.notified().await;
                    Err(close_error("expected cleanup failure"))
                }
            },
        );

        let first_activation = tokio::spawn({
            let handle = handle.clone();
            async move { handle.activate().await }
        });
        let second_activation = tokio::spawn({
            let handle = handle.clone();
            async move { handle.activate().await }
        });
        first_activation.await.unwrap().unwrap();
        second_activation.await.unwrap().unwrap();
        assert_eq!(activations.load(Ordering::SeqCst), 1);

        let first_close = tokio::spawn({
            let handle = handle.clone();
            async move { handle.close().await }
        });
        let second_close = tokio::spawn({
            let handle = handle.clone();
            async move { handle.close().await }
        });
        close_started.notified().await;
        close_release.notify_waiters();
        let first = first_close.await.unwrap().unwrap_err();
        let second = second_close.await.unwrap().unwrap_err();
        assert_eq!(first.to_string(), second.to_string());
        assert_eq!(cleanups.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn activation_error_is_preserved_and_cleanup_runs_once() {
        let cleanups = Arc::new(AtomicUsize::new(0));
        let activation_error = RegistryError::message(
            RegistryOperation::ActivateRegistration,
            RegistryErrorKind::Unavailable,
            "expected activation failure",
        );
        let handle = prepare_registration(
            {
                let activation_error = activation_error.clone();
                async move { Err(activation_error) }
            },
            {
                let cleanups = cleanups.clone();
                move || async move {
                    cleanups.fetch_add(1, Ordering::SeqCst);
                    Ok(())
                }
            },
        );

        let error = handle.activate().await.unwrap_err();
        assert_eq!(error.kind(), RegistryErrorKind::Unavailable);
        handle.close().await.unwrap();
        handle.close().await.unwrap();
        assert_eq!(cleanups.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn activation_panic_is_isolated_and_cleanup_still_runs_once() {
        let cleanups = Arc::new(AtomicUsize::new(0));
        let handle = prepare_registration(
            async {
                panic!("expected provider activation panic");
            },
            {
                let cleanups = cleanups.clone();
                move || async move {
                    cleanups.fetch_add(1, Ordering::SeqCst);
                    Ok(())
                }
            },
        );

        let error = handle.activate().await.unwrap_err();
        assert_eq!(error.kind(), RegistryErrorKind::Internal);
        handle.close().await.unwrap();
        handle.close().await.unwrap();
        assert_eq!(cleanups.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn cleanup_panic_is_isolated_and_shared() {
        let cleanups = Arc::new(AtomicUsize::new(0));
        let handle = prepare_registration(async { Ok(()) }, {
            let cleanups = cleanups.clone();
            move || async move {
                cleanups.fetch_add(1, Ordering::SeqCst);
                panic!("expected provider cleanup panic");
            }
        });
        handle.activate().await.unwrap();

        let first = handle.close().await.unwrap_err();
        let second = handle.close().await.unwrap_err();
        assert_eq!(first.kind(), RegistryErrorKind::Internal);
        assert_eq!(first.to_string(), second.to_string());
        assert_eq!(cleanups.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn subscription_activation_returns_the_prepared_directory() {
        let (publisher, directory) = directory();
        let expected = directory.clone();
        let handle = prepare_subscription(
            directory,
            async move {
                publisher.publish_ready(Vec::new())?;
                Ok(())
            },
            || async { Ok(()) },
        );

        let active = handle.activate().await.unwrap();
        assert_eq!(active.snapshot().revision(), expected.snapshot().revision());
        assert_eq!(active.snapshot().state(), expected.snapshot().state());
        handle.close().await.unwrap();
    }

    #[tokio::test]
    async fn last_handle_drop_only_requests_background_close() {
        let completed = Arc::new(Notify::new());
        let cleanups = Arc::new(AtomicUsize::new(0));
        let handle = prepare_registration(async { Ok(()) }, {
            let completed = completed.clone();
            let cleanups = cleanups.clone();
            move || async move {
                cleanups.fetch_add(1, Ordering::SeqCst);
                completed.notify_one();
                Ok(())
            }
        });
        handle.activate().await.unwrap();
        drop(handle);

        completed.notified().await;
        assert_eq!(cleanups.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn close_before_activation_is_ready_without_a_runtime() {
        let handle = prepare_registration(async { Ok(()) }, || async { Ok(()) });
        let mut future = handle.close();
        let waker = Waker::noop();
        let mut context = Context::from_waker(waker);
        assert!(matches!(
            Pin::as_mut(&mut future).poll(&mut context),
            Poll::Ready(Ok(()))
        ));
    }
}
