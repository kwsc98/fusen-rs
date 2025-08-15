use fusen_internal_common::resource::service::ServiceResource;
use std::sync::Arc;
use tokio::sync::{
    mpsc::{self, UnboundedSender},
    oneshot,
};

use crate::error::RegisterError;

#[derive(Debug)]
pub enum DirectorySender {
    GET,
    CHANGE(Vec<ServiceResource>),
}

pub enum DirectoryReceiver {
    GET(Arc<Vec<Arc<ServiceResource>>>),
    CHANGE,
}

#[derive(Clone, Debug)]
pub struct Directory {
    sender: UnboundedSender<(DirectorySender, oneshot::Sender<DirectoryReceiver>)>,
}

impl Default for Directory {
    fn default() -> Self {
        let (s, mut r) =
            mpsc::unbounded_channel::<(DirectorySender, oneshot::Sender<DirectoryReceiver>)>();
        tokio::spawn(async move {
            let mut cache: Arc<Vec<Arc<ServiceResource>>> = Arc::new(vec![]);
            while let Some(msg) = r.recv().await {
                match msg.0 {
                    DirectorySender::GET => {
                        let _ = msg.1.send(DirectoryReceiver::GET(cache.clone()));
                    }
                    DirectorySender::CHANGE(resources) => {
                        cache = Arc::new(resources.into_iter().map(|e| Arc::new(e)).collect());
                        let _ = msg.1.send(DirectoryReceiver::CHANGE);
                    }
                }
            }
        });
        Self { sender: s }
    }
}

impl Directory {
    pub async fn get(&self) -> Result<Arc<Vec<Arc<ServiceResource>>>, RegisterError> {
        let oneshot = oneshot::channel();
        let _ = self.sender.send((DirectorySender::GET, oneshot.0));
        let rev = oneshot
            .1
            .await
            .map_err(|e| RegisterError::Error(Box::new(e)))?;
        match rev {
            DirectoryReceiver::GET(rev) => Ok(rev),
            DirectoryReceiver::CHANGE => Err(RegisterError::Impossible),
        }
    }

    pub async fn change(&self, resource: Vec<ServiceResource>) -> Result<(), RegisterError> {
        let oneshot = oneshot::channel();
        let _ = self
            .sender
            .send((DirectorySender::CHANGE(resource), oneshot.0));
        let rev = oneshot
            .1
            .await
            .map_err(|e| RegisterError::Error(Box::new(e)))?;
        match rev {
            DirectoryReceiver::GET(_) => Err(RegisterError::Impossible),
            DirectoryReceiver::CHANGE => Ok(()),
        }
    }
}
