use crate::error::RegisterError;
use fusen_internal_common::resource::service::ServiceResource;
use std::sync::Arc;
use tokio::sync::watch;

/// A cheap, cloneable snapshot of the currently discovered service instances.
#[derive(Clone, Debug)]
pub struct Directory {
    sender: watch::Sender<Arc<Vec<Arc<ServiceResource>>>>,
    receiver: watch::Receiver<Arc<Vec<Arc<ServiceResource>>>>,
}

impl Default for Directory {
    fn default() -> Self {
        let (sender, receiver) = watch::channel(Arc::new(Vec::new()));
        Self { sender, receiver }
    }
}

impl Directory {
    /// Returns the latest service snapshot without actor round trips.
    pub fn snapshot(&self) -> Arc<Vec<Arc<ServiceResource>>> {
        self.receiver.borrow().clone()
    }

    /// Compatibility async accessor for callers that already await directory reads.
    pub async fn get(&self) -> Result<Arc<Vec<Arc<ServiceResource>>>, RegisterError> {
        Ok(self.snapshot())
    }

    /// Atomically replaces all discovered instances and notifies subscribers.
    pub fn replace(&self, resources: Vec<ServiceResource>) -> Result<(), RegisterError> {
        self.sender
            .send(Arc::new(resources.into_iter().map(Arc::new).collect()))
            .map_err(|_| RegisterError::DirectoryClosed)
    }

    /// Compatibility async updater.
    pub async fn change(&self, resources: Vec<ServiceResource>) -> Result<(), RegisterError> {
        self.replace(resources)
    }

    /// Waits until a newer snapshot is available.
    pub async fn changed(&mut self) -> Result<(), RegisterError> {
        self.receiver
            .changed()
            .await
            .map_err(|_| RegisterError::DirectoryClosed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn replaces_snapshot() {
        let directory = Directory::default();
        directory
            .replace(vec![ServiceResource {
                service_id: "demo".into(),
                group: None,
                version: None,
                methods: Vec::new(),
                addr: "http://127.0.0.1:8080".into(),
                weight: Some(1.0),
                metadata: Default::default(),
            }])
            .unwrap();
        assert_eq!(directory.snapshot().len(), 1);
    }
}
