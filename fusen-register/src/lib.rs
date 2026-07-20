#![warn(missing_docs)]
//! Service registration and discovery contracts for fusen-rs.

use fusen_internal_common::{
    BoxFuture, protocol::WireProtocol, resource::service::ServiceResource,
};
use std::sync::atomic::{AtomicBool, Ordering};
use std::{future::Future, sync::Arc};
use tokio::sync::watch;

use crate::{directory::Directory, error::RegisterError};

/// The caller-owned half of one shared subscription cleanup operation.
#[derive(Clone)]
pub struct SubscriptionCloser {
    request: watch::Sender<bool>,
    completion: watch::Receiver<Option<Result<(), RegisterError>>>,
}

/// The provider-owned half of one shared subscription cleanup operation.
#[must_use = "the cleanup future must be run by the registry provider"]
pub struct SubscriptionCleanup {
    request: watch::Receiver<bool>,
    completion: watch::Sender<Option<Result<(), RegisterError>>>,
}

/// Creates the caller and provider halves of one cleanup operation.
pub fn subscription_cleanup() -> (SubscriptionCloser, SubscriptionCleanup) {
    let (request, request_receiver) = watch::channel(false);
    let (completion, completion_receiver) = watch::channel(None);
    (
        SubscriptionCloser {
            request,
            completion: completion_receiver,
        },
        SubscriptionCleanup {
            request: request_receiver,
            completion,
        },
    )
}

impl SubscriptionCloser {
    fn completed(result: Result<(), RegisterError>) -> Self {
        let (request, _) = watch::channel(true);
        let (completion_sender, completion) = watch::channel(Some(result));
        drop(completion_sender);
        Self {
            request,
            completion,
        }
    }

    /// Requests provider cleanup without blocking the caller.
    pub fn request_close(&self) {
        self.request.send_replace(true);
    }

    async fn wait_closed(&self) -> Result<(), RegisterError> {
        let mut completion = self.completion.clone();
        loop {
            if let Some(result) = completion.borrow().clone() {
                return result;
            }
            completion
                .changed()
                .await
                .map_err(|_| RegisterError::CleanupAborted)?;
        }
    }
}

impl SubscriptionCleanup {
    /// Waits for a close request, runs provider cleanup, and publishes its result.
    ///
    /// Dropping or unwinding this future before completion publishes
    /// [`RegisterError::CleanupAborted`] to every waiter.
    pub async fn run<F>(mut self, cleanup: F)
    where
        F: Future<Output = Result<(), RegisterError>> + Send,
    {
        let mut completion = CompletionGuard::new(self.completion);
        if !*self.request.borrow() {
            let _ = self.request.changed().await;
        }
        completion.complete(cleanup.await);
    }
}

struct CompletionGuard {
    completion: Option<watch::Sender<Option<Result<(), RegisterError>>>>,
}

impl CompletionGuard {
    fn new(completion: watch::Sender<Option<Result<(), RegisterError>>>) -> Self {
        Self {
            completion: Some(completion),
        }
    }

    fn complete(&mut self, result: Result<(), RegisterError>) {
        if let Some(completion) = self.completion.take() {
            completion.send_replace(Some(result));
        }
    }
}

impl Drop for CompletionGuard {
    fn drop(&mut self) {
        if let Some(completion) = self.completion.take() {
            completion.send_replace(Some(Err(RegisterError::CleanupAborted)));
        }
    }
}

struct SubscriptionInner {
    directory: Directory,
    closer: SubscriptionCloser,
    close_requested: AtomicBool,
}

impl SubscriptionInner {
    fn request_close(&self) {
        if self
            .close_requested
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
        {
            self.closer.request_close();
        }
    }
}

impl Drop for SubscriptionInner {
    fn drop(&mut self) {
        self.request_close();
    }
}

/// A discovered instance directory paired with explicit subscription cleanup.
#[derive(Clone)]
pub struct ServiceSubscription {
    inner: Arc<SubscriptionInner>,
}

impl ServiceSubscription {
    /// Creates a provider-backed subscription.
    pub fn new(directory: Directory, closer: SubscriptionCloser) -> Self {
        Self {
            inner: Arc::new(SubscriptionInner {
                directory,
                closer,
                close_requested: AtomicBool::new(false),
            }),
        }
    }

    /// Creates an immutable local directory with no remote cleanup.
    pub fn local(resources: Vec<ServiceResource>) -> Self {
        Self::new(
            Directory::fixed(resources),
            SubscriptionCloser::completed(Ok(())),
        )
    }

    /// Returns the current read-only discovered instance directory.
    pub fn directory(&self) -> &Directory {
        &self.inner.directory
    }

    /// Closes the remote subscription and shares one cleanup result with all callers.
    pub async fn close(&self) -> Result<(), RegisterError> {
        self.inner.request_close();
        self.inner.closer.wait_closed().await
    }
}

/// Atomic service instance snapshots.
pub mod directory;
/// Registration and directory failures.
pub mod error;
/// Shared types used when implementing [`Register`].
pub use fusen_internal_common;

/// Pluggable service registry used by clients and servers.
pub trait Register: Send + Sync {
    /// Publishes one service instance. Implementations must be idempotent.
    ///
    /// A caller may compensate with [`Register::deregister`] when the result is uncertain,
    /// including after a local timeout or cancellation.
    fn register(
        &self,
        resource: Arc<ServiceResource>,
        protocol: WireProtocol,
    ) -> BoxFuture<Result<(), RegisterError>>;

    /// Removes one previously published service instance. Implementations must be idempotent,
    /// including when no matching instance exists.
    fn deregister(
        &self,
        resource: Arc<ServiceResource>,
        protocol: WireProtocol,
    ) -> BoxFuture<Result<(), RegisterError>>;

    /// Subscribes to all instances matching the requested service resource.
    fn subscribe(
        &self,
        resource: ServiceResource,
        protocol: WireProtocol,
    ) -> BoxFuture<Result<ServiceSubscription, RegisterError>>;
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        pin::Pin,
        sync::atomic::{AtomicUsize, Ordering},
        task::{Context, Poll, Waker},
        time::Duration,
    };
    use tokio::sync::Notify;

    #[tokio::test]
    async fn concurrent_close_shares_one_cleanup_result() {
        let (closer, cleanup) = subscription_cleanup();
        let release = Arc::new(Notify::new());
        let cleanup_count = Arc::new(AtomicUsize::new(0));
        tokio::spawn(cleanup.run({
            let release = release.clone();
            let cleanup_count = cleanup_count.clone();
            async move {
                cleanup_count.fetch_add(1, Ordering::SeqCst);
                release.notified().await;
                Err(RegisterError::InvalidResource("cleanup failed".into()))
            }
        }));
        let subscription = ServiceSubscription::new(Directory::fixed(Vec::new()), closer);
        let first = tokio::spawn({
            let subscription = subscription.clone();
            async move { subscription.close().await }
        });
        let second = tokio::spawn({
            let subscription = subscription.clone();
            async move { subscription.close().await }
        });
        tokio::time::timeout(Duration::from_secs(1), async {
            while cleanup_count.load(Ordering::SeqCst) == 0 {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        release.notify_waiters();
        for result in [first.await.unwrap(), second.await.unwrap()] {
            assert!(matches!(
                result,
                Err(RegisterError::InvalidResource(message)) if message == "cleanup failed"
            ));
        }
        assert_eq!(cleanup_count.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn cancelling_waiter_does_not_cancel_cleanup() {
        let (closer, cleanup) = subscription_cleanup();
        let started = Arc::new(Notify::new());
        let release = Arc::new(Notify::new());
        tokio::spawn(cleanup.run({
            let started = started.clone();
            let release = release.clone();
            async move {
                started.notify_one();
                release.notified().await;
                Ok(())
            }
        }));
        let subscription = ServiceSubscription::new(Directory::fixed(Vec::new()), closer);
        let first = tokio::spawn({
            let subscription = subscription.clone();
            async move { subscription.close().await }
        });
        started.notified().await;
        first.abort();
        release.notify_waiters();
        subscription.close().await.unwrap();
    }

    #[tokio::test]
    async fn aborted_cleanup_publishes_terminal_error() {
        let (closer, cleanup) = subscription_cleanup();
        let started = Arc::new(Notify::new());
        let task = tokio::spawn(cleanup.run({
            let started = started.clone();
            async move {
                started.notify_one();
                std::future::pending().await
            }
        }));
        let subscription = ServiceSubscription::new(Directory::fixed(Vec::new()), closer);
        let waiter = tokio::spawn({
            let subscription = subscription.clone();
            async move { subscription.close().await }
        });
        started.notified().await;
        task.abort();
        assert!(matches!(
            waiter.await.unwrap(),
            Err(RegisterError::CleanupAborted)
        ));
    }

    #[tokio::test]
    async fn dropped_cleanup_before_start_publishes_terminal_error() {
        let (closer, cleanup) = subscription_cleanup();
        drop(cleanup);
        assert!(matches!(
            closer.wait_closed().await,
            Err(RegisterError::CleanupAborted)
        ));
    }

    #[tokio::test]
    async fn panicking_cleanup_publishes_terminal_error() {
        let (closer, cleanup) = subscription_cleanup();
        let task = tokio::spawn(cleanup.run(async move {
            panic!("cleanup panic");
            #[allow(unreachable_code)]
            Ok(())
        }));
        let subscription = ServiceSubscription::new(Directory::fixed(Vec::new()), closer);
        let result = subscription.close().await;
        assert!(task.await.unwrap_err().is_panic());
        assert!(matches!(result, Err(RegisterError::CleanupAborted)));
    }

    #[tokio::test]
    async fn last_drop_runs_cleanup_once() {
        let (closer, cleanup) = subscription_cleanup();
        let cleanup_count = Arc::new(AtomicUsize::new(0));
        let completed = Arc::new(Notify::new());
        tokio::spawn(cleanup.run({
            let cleanup_count = cleanup_count.clone();
            let completed = completed.clone();
            async move {
                cleanup_count.fetch_add(1, Ordering::SeqCst);
                completed.notify_one();
                Ok(())
            }
        }));
        let subscription = ServiceSubscription::new(Directory::fixed(Vec::new()), closer);
        let clone = subscription.clone();
        drop(subscription);
        assert_eq!(cleanup_count.load(Ordering::SeqCst), 0);
        drop(clone);
        tokio::time::timeout(Duration::from_secs(1), completed.notified())
            .await
            .unwrap();
        assert_eq!(cleanup_count.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn local_close_is_ready_without_a_tokio_runtime() {
        let subscription = ServiceSubscription::local(Vec::new());
        let mut future = Box::pin(subscription.close());
        let waker = Waker::noop();
        let mut context = Context::from_waker(waker);
        assert!(matches!(
            Pin::as_mut(&mut future).poll(&mut context),
            Poll::Ready(Ok(()))
        ));
    }
}
