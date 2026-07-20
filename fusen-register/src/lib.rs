#![warn(missing_docs)]
//! Service registration and discovery contracts for fusen-rs.

use fusen_internal_common::{
    BoxFuture, protocol::WireProtocol, resource::service::ServiceResource,
};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use tokio::sync::watch;

use crate::{directory::Directory, error::RegisterError};

/// Provider-owned cleanup for one live discovery subscription.
pub trait SubscriptionLifecycle: Send + Sync {
    /// Requests non-blocking cleanup, primarily from [`Drop`].
    fn request_close(&self);

    /// Waits for provider cleanup to finish.
    fn close(&self) -> BoxFuture<Result<(), RegisterError>>;
}

struct NoopSubscription;

impl SubscriptionLifecycle for NoopSubscription {
    fn request_close(&self) {}

    fn close(&self) -> BoxFuture<Result<(), RegisterError>> {
        Box::pin(async { Ok(()) })
    }
}

struct SubscriptionInner {
    directory: Directory,
    lifecycle: Arc<dyn SubscriptionLifecycle>,
    close_started: AtomicBool,
    close_result: watch::Sender<Option<Result<(), RegisterError>>>,
}

impl Drop for SubscriptionInner {
    fn drop(&mut self) {
        if self
            .close_started
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
        {
            self.lifecycle.request_close();
        }
    }
}

impl SubscriptionInner {
    fn start_close(&self) {
        if self
            .close_started
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            return;
        }
        self.lifecycle.request_close();
        let lifecycle = self.lifecycle.clone();
        let close_result = self.close_result.clone();
        tokio::spawn(async move {
            close_result.send_replace(Some(lifecycle.close().await));
        });
    }
}

/// A discovered instance directory paired with explicit subscription cleanup.
#[derive(Clone)]
pub struct ServiceSubscription {
    inner: Arc<SubscriptionInner>,
}

impl ServiceSubscription {
    /// Creates a provider-backed subscription.
    pub fn new(directory: Directory, lifecycle: Arc<dyn SubscriptionLifecycle>) -> Self {
        let (close_result, _) = watch::channel(None);
        Self {
            inner: Arc::new(SubscriptionInner {
                directory,
                lifecycle,
                close_started: AtomicBool::new(false),
                close_result,
            }),
        }
    }

    /// Creates an immutable local directory with no remote cleanup.
    pub fn local(directory: Directory) -> Self {
        Self::new(directory, Arc::new(NoopSubscription))
    }

    /// Returns the current discovered instance directory.
    pub fn directory(&self) -> &Directory {
        &self.inner.directory
    }

    /// Closes the remote subscription and shares one cleanup result with all callers.
    pub async fn close(&self) -> Result<(), RegisterError> {
        self.inner.start_close();
        let mut close_result = self.inner.close_result.subscribe();
        loop {
            if let Some(result) = close_result.borrow().clone() {
                return result;
            }
            close_result
                .changed()
                .await
                .expect("subscription close result sender is retained by the subscription");
        }
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
    use std::sync::atomic::AtomicUsize;
    use std::time::Duration;
    use tokio::sync::Notify;

    async fn wait_for_cleanup_start(closed: &AtomicUsize) {
        tokio::time::timeout(Duration::from_secs(1), async {
            while closed.load(Ordering::SeqCst) == 0 {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("subscription cleanup did not start");
    }

    struct MockLifecycle {
        requested: Arc<AtomicUsize>,
        closed: Arc<AtomicUsize>,
        release: Option<Arc<Notify>>,
        result: Result<(), RegisterError>,
    }

    impl SubscriptionLifecycle for MockLifecycle {
        fn request_close(&self) {
            self.requested.fetch_add(1, Ordering::SeqCst);
        }

        fn close(&self) -> BoxFuture<Result<(), RegisterError>> {
            let closed = self.closed.clone();
            let release = self.release.clone();
            let result = self.result.clone();
            Box::pin(async move {
                closed.fetch_add(1, Ordering::SeqCst);
                if let Some(release) = release {
                    release.notified().await;
                }
                result
            })
        }
    }

    #[tokio::test]
    async fn concurrent_close_is_idempotent_and_shares_the_result() {
        let requested = Arc::new(AtomicUsize::new(0));
        let closed = Arc::new(AtomicUsize::new(0));
        let release = Arc::new(Notify::new());
        let subscription = ServiceSubscription::new(
            Directory::default(),
            Arc::new(MockLifecycle {
                requested: requested.clone(),
                closed: closed.clone(),
                release: Some(release.clone()),
                result: Err(RegisterError::InvalidResource("cleanup failed".into())),
            }),
        );
        let first = tokio::spawn({
            let subscription = subscription.clone();
            async move { subscription.close().await }
        });
        let second = tokio::spawn({
            let subscription = subscription.clone();
            async move { subscription.close().await }
        });
        wait_for_cleanup_start(&closed).await;
        release.notify_waiters();
        assert!(matches!(
            first.await.unwrap(),
            Err(RegisterError::InvalidResource(message)) if message == "cleanup failed"
        ));
        assert!(matches!(
            second.await.unwrap(),
            Err(RegisterError::InvalidResource(message)) if message == "cleanup failed"
        ));
        assert_eq!(closed.load(Ordering::SeqCst), 1);
        drop(subscription);
        assert_eq!(requested.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn cancelling_first_waiter_does_not_cancel_cleanup() {
        let requested = Arc::new(AtomicUsize::new(0));
        let closed = Arc::new(AtomicUsize::new(0));
        let release = Arc::new(Notify::new());
        let subscription = ServiceSubscription::new(
            Directory::default(),
            Arc::new(MockLifecycle {
                requested: requested.clone(),
                closed: closed.clone(),
                release: Some(release.clone()),
                result: Ok(()),
            }),
        );
        let first = tokio::spawn({
            let subscription = subscription.clone();
            async move { subscription.close().await }
        });
        wait_for_cleanup_start(&closed).await;
        first.abort();
        release.notify_waiters();
        subscription.close().await.unwrap();
        assert_eq!(requested.load(Ordering::SeqCst), 1);
        assert_eq!(closed.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn last_drop_requests_cleanup_once() {
        let requested = Arc::new(AtomicUsize::new(0));
        let subscription = ServiceSubscription::new(
            Directory::default(),
            Arc::new(MockLifecycle {
                requested: requested.clone(),
                closed: Arc::new(AtomicUsize::new(0)),
                release: None,
                result: Ok(()),
            }),
        );
        let clone = subscription.clone();
        drop(subscription);
        assert_eq!(requested.load(Ordering::SeqCst), 0);
        drop(clone);
        assert_eq!(requested.load(Ordering::SeqCst), 1);
    }
}
