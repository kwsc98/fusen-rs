use crate::error::{RegistryError, RegistryErrorKind, RegistryOperation};
use fusen_contract::ServiceInstance;
use std::{
    ops::Deref,
    sync::{Arc, Mutex},
    time::Instant,
};
use tokio::sync::watch;

/// Provider health associated with one immutable service snapshot.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum DirectoryState {
    /// The provider has not published its initial snapshot.
    Initializing,
    /// The snapshot is current and may be routed.
    Ready,
    /// The provider is reconnecting and the retained snapshot may be stale.
    Stale,
    /// The provider cannot currently supply a routable snapshot.
    Unavailable,
    /// The subscription publisher has terminated.
    Closed,
}

/// One immutable, monotonically versioned service discovery snapshot.
#[derive(Clone, Debug)]
pub struct DirectorySnapshot {
    revision: u64,
    observed_at: Instant,
    state: DirectoryState,
    instances: Arc<[ServiceInstance]>,
}

impl DirectorySnapshot {
    /// Returns the provider-local monotonically increasing revision.
    pub const fn revision(&self) -> u64 {
        self.revision
    }

    /// Returns when this snapshot or provider state was observed.
    pub const fn observed_at(&self) -> Instant {
        self.observed_at
    }

    /// Returns the provider state associated with this snapshot.
    pub const fn state(&self) -> DirectoryState {
        self.state
    }

    /// Returns the immutable service instances retained by this snapshot.
    pub fn instances(&self) -> &[ServiceInstance] {
        &self.instances
    }

    /// Clones the shared instance allocation without copying individual instances.
    pub fn shared_instances(&self) -> Arc<[ServiceInstance]> {
        self.instances.clone()
    }
}

impl Deref for DirectorySnapshot {
    type Target = [ServiceInstance];

    fn deref(&self) -> &Self::Target {
        self.instances()
    }
}

/// A cloneable, read-only view of latest-wins service discovery snapshots.
#[derive(Clone, Debug)]
pub struct Directory {
    receiver: watch::Receiver<DirectorySnapshot>,
}

/// Provider-owned publication access for one [`Directory`].
///
/// The Tokio channel remains private; publishers can only replace the latest immutable snapshot.
#[derive(Clone, Debug)]
pub struct DirectoryPublisher {
    inner: Arc<PublisherInner>,
}

#[derive(Debug)]
struct PublisherInner {
    state: Mutex<PublisherState>,
}

#[derive(Debug)]
struct PublisherState {
    revision: u64,
    sender: watch::Sender<DirectorySnapshot>,
}

/// Creates an initializing directory and its provider-owned publisher.
pub fn directory() -> (DirectoryPublisher, Directory) {
    let snapshot = DirectorySnapshot {
        revision: 0,
        observed_at: Instant::now(),
        state: DirectoryState::Initializing,
        instances: Arc::from([]),
    };
    let (sender, receiver) = watch::channel(snapshot);
    (
        DirectoryPublisher {
            inner: Arc::new(PublisherInner {
                state: Mutex::new(PublisherState {
                    revision: 0,
                    sender,
                }),
            }),
        },
        Directory { receiver },
    )
}

impl Directory {
    /// Returns the latest snapshot without an actor round trip.
    pub fn snapshot(&self) -> DirectorySnapshot {
        self.receiver.borrow().clone()
    }

    /// Waits for and returns a newer snapshot.
    pub async fn changed(&mut self) -> Result<DirectorySnapshot, RegistryError> {
        self.receiver.changed().await.map_err(|_| {
            RegistryError::message(
                RegistryOperation::Directory,
                RegistryErrorKind::Unavailable,
                "directory publisher ended",
            )
        })?;
        Ok(self.snapshot())
    }
}

impl DirectoryPublisher {
    /// Publishes a ready snapshot and replaces all retained instances.
    pub fn publish_ready(
        &self,
        instances: Vec<ServiceInstance>,
    ) -> Result<DirectorySnapshot, RegistryError> {
        self.publish(DirectoryState::Ready, Some(Arc::from(instances)))
    }

    /// Publishes one provider snapshot atomically, replacing both state and instances.
    ///
    /// This is useful for latest-wins forwarding where an observer may legitimately skip the
    /// provider's preceding `Ready` revision and first observe its retained `Stale` snapshot.
    pub fn publish_snapshot(
        &self,
        state: DirectoryState,
        instances: Vec<ServiceInstance>,
    ) -> Result<DirectorySnapshot, RegistryError> {
        self.publish(state, Some(Arc::from(instances)))
    }

    /// Publishes provider state while retaining the latest instance allocation.
    pub fn publish_state(&self, state: DirectoryState) -> Result<DirectorySnapshot, RegistryError> {
        self.publish(state, None)
    }

    /// Returns true when no directory readers remain.
    pub fn is_closed(&self) -> bool {
        self.inner
            .state
            .lock()
            .map(|state| state.sender.is_closed())
            .unwrap_or(true)
    }

    fn publish(
        &self,
        state: DirectoryState,
        instances: Option<Arc<[ServiceInstance]>>,
    ) -> Result<DirectorySnapshot, RegistryError> {
        if state == DirectoryState::Initializing || state == DirectoryState::Closed {
            return Err(RegistryError::message(
                RegistryOperation::Directory,
                RegistryErrorKind::InvalidResource,
                "Initializing and Closed are lifecycle-managed directory states",
            ));
        }
        let mut publisher = self.inner.state.lock().map_err(|_| {
            RegistryError::message(
                RegistryOperation::Directory,
                RegistryErrorKind::Internal,
                "directory publisher lock was poisoned",
            )
        })?;
        publisher.revision = publisher.revision.checked_add(1).ok_or_else(|| {
            RegistryError::message(
                RegistryOperation::Directory,
                RegistryErrorKind::Internal,
                "directory revision overflowed",
            )
        })?;
        let instances = instances.unwrap_or_else(|| publisher.sender.borrow().shared_instances());
        let snapshot = DirectorySnapshot {
            revision: publisher.revision,
            observed_at: Instant::now(),
            state,
            instances,
        };
        publisher.sender.send_replace(snapshot.clone());
        Ok(snapshot)
    }
}

impl Drop for PublisherInner {
    fn drop(&mut self) {
        let Ok(publisher) = self.state.get_mut() else {
            return;
        };
        let Some(revision) = publisher.revision.checked_add(1) else {
            return;
        };
        publisher.revision = revision;
        let snapshot = DirectorySnapshot {
            revision,
            observed_at: Instant::now(),
            state: DirectoryState::Closed,
            instances: publisher.sender.borrow().shared_instances(),
        };
        publisher.sender.send_replace(snapshot);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use fusen_contract::{InstanceId, Metadata};

    fn instance(addr: &str) -> ServiceInstance {
        ServiceInstance::new(
            InstanceId::new("test-instance").unwrap(),
            addr.parse().unwrap(),
            Default::default(),
            Default::default(),
        )
    }

    #[tokio::test]
    async fn snapshots_are_latest_wins_and_revisions_are_monotonic() {
        let (publisher, mut directory) = directory();
        assert_eq!(directory.snapshot().revision(), 0);
        assert_eq!(directory.snapshot().state(), DirectoryState::Initializing);

        publisher
            .publish_ready(vec![instance("http://old")])
            .unwrap();
        publisher
            .publish_ready(vec![instance("http://latest")])
            .unwrap();
        let snapshot = directory.changed().await.unwrap();
        assert_eq!(snapshot.revision(), 2);
        assert_eq!(snapshot.state(), DirectoryState::Ready);
        assert_eq!(snapshot[0].endpoint().as_url().host_str(), Some("latest"));

        let stale = publisher.publish_state(DirectoryState::Stale).unwrap();
        assert_eq!(stale.revision(), 3);
        assert!(Arc::ptr_eq(
            &snapshot.shared_instances(),
            &stale.shared_instances()
        ));

        let unavailable = publisher
            .publish_snapshot(
                DirectoryState::Unavailable,
                vec![instance("http://forwarded")],
            )
            .unwrap();
        assert_eq!(unavailable.revision(), 4);
        assert_eq!(unavailable.state(), DirectoryState::Unavailable);
        assert_eq!(
            unavailable[0].endpoint().as_url().host_str(),
            Some("forwarded")
        );
    }

    #[tokio::test]
    async fn last_publisher_drop_publishes_one_closed_snapshot() {
        let (publisher, mut directory) = directory();
        let clone = publisher.clone();
        publisher.publish_ready(Vec::new()).unwrap();
        let _ = directory.changed().await.unwrap();
        drop(publisher);
        assert_eq!(directory.snapshot().state(), DirectoryState::Ready);
        drop(clone);

        let closed = directory.changed().await.unwrap();
        assert_eq!(closed.state(), DirectoryState::Closed);
        assert_eq!(closed.revision(), 2);
        assert!(directory.changed().await.is_err());
    }

    #[test]
    fn lifecycle_managed_states_cannot_be_published_directly() {
        let (publisher, _) = directory();
        for state in [DirectoryState::Initializing, DirectoryState::Closed] {
            let error = publisher.publish_state(state).unwrap_err();
            assert_eq!(error.kind(), RegistryErrorKind::InvalidResource);
        }
    }

    #[test]
    fn directory_debug_never_expands_instance_metadata() {
        let (publisher, directory) = directory();
        let instance = instance("http://provider")
            .with_metadata(Metadata::from([(
                "credential".into(),
                "private-directory-token".into(),
            )]))
            .unwrap();
        let snapshot = publisher.publish_ready(vec![instance]).unwrap();

        for debug in [
            format!("{snapshot:?}"),
            format!("{directory:?}"),
            format!("{publisher:?}"),
        ] {
            assert!(!debug.contains("private-directory-token"));
        }
    }
}
