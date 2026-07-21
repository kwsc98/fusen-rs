use crate::error::RegisterError;
use fusen_contract::ServiceInstance;
use std::sync::Arc;
use tokio::sync::watch;

/// One immutable service discovery snapshot.
pub type ServiceSnapshot = Arc<Vec<Arc<ServiceInstance>>>;

/// A cloneable, read-only view of atomic service instance snapshots.
#[derive(Clone, Debug)]
pub struct Directory {
    receiver: watch::Receiver<ServiceSnapshot>,
}

/// Provider-owned write access to one [`Directory`].
#[derive(Clone, Debug)]
pub struct DirectoryWriter {
    sender: watch::Sender<ServiceSnapshot>,
}

/// Creates a provider writer and a consumer directory with an initial snapshot.
pub fn directory_channel(instances: Vec<ServiceInstance>) -> (DirectoryWriter, Directory) {
    let snapshot = to_snapshot(instances);
    let (sender, receiver) = watch::channel(snapshot);
    (DirectoryWriter { sender }, Directory { receiver })
}

impl Directory {
    /// Creates a read-only directory that will never receive updates.
    pub fn fixed(instances: Vec<ServiceInstance>) -> Self {
        let (_, directory) = directory_channel(instances);
        directory
    }

    /// Returns the latest service snapshot without actor round trips.
    pub fn snapshot(&self) -> ServiceSnapshot {
        self.receiver.borrow().clone()
    }

    /// Waits for a newer snapshot and returns it.
    ///
    /// Returns [`RegisterError::DirectoryClosed`] after every writer is dropped.
    pub async fn changed(&mut self) -> Result<ServiceSnapshot, RegisterError> {
        self.receiver
            .changed()
            .await
            .map_err(|_| RegisterError::DirectoryClosed)?;
        Ok(self.snapshot())
    }
}

impl DirectoryWriter {
    /// Atomically replaces the snapshot, retaining the latest value even without readers.
    pub fn replace(&self, instances: Vec<ServiceInstance>) {
        self.sender.send_replace(to_snapshot(instances));
    }

    /// Returns true when no directory readers remain.
    pub fn is_closed(&self) -> bool {
        self.sender.is_closed()
    }
}

fn to_snapshot(instances: Vec<ServiceInstance>) -> ServiceSnapshot {
    Arc::new(instances.into_iter().map(Arc::new).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn instance(addr: &str) -> ServiceInstance {
        ServiceInstance::new(addr.parse().unwrap(), Default::default())
    }

    #[tokio::test]
    async fn writer_replaces_snapshot_and_notifies_reader() {
        let (writer, mut directory) = directory_channel(vec![instance("http://old")]);
        writer.replace(vec![instance("http://new")]);
        let snapshot = directory.changed().await.unwrap();
        assert_eq!(snapshot[0].endpoint().as_url().host_str(), Some("new"));
    }

    #[tokio::test]
    async fn updates_are_latest_wins() {
        let (writer, mut directory) = directory_channel(Vec::new());
        writer.replace(vec![instance("http://old")]);
        writer.replace(vec![instance("http://latest")]);
        let snapshot = directory.changed().await.unwrap();
        assert_eq!(snapshot[0].endpoint().as_url().host_str(), Some("latest"));
    }

    #[tokio::test]
    async fn reader_observes_directory_closure() {
        let (writer, mut directory) = directory_channel(Vec::new());
        assert!(!writer.is_closed());
        drop(writer);
        assert!(matches!(
            directory.changed().await,
            Err(RegisterError::DirectoryClosed)
        ));
    }

    #[tokio::test]
    async fn fixed_directory_retains_snapshot_and_is_closed_for_updates() {
        let mut directory = Directory::fixed(vec![instance("http://fixed")]);
        assert_eq!(
            directory.snapshot()[0].endpoint().as_url().host_str(),
            Some("fixed")
        );
        assert!(matches!(
            directory.changed().await,
            Err(RegisterError::DirectoryClosed)
        ));
    }
}
