use std::sync::Arc;
use tokio::sync::RwLock;

#[derive(Clone)]
pub struct AsyncCache<V> {
    value: Arc<RwLock<Option<V>>>,
}

impl<V> Default for AsyncCache<V>
where
    V: Send + Sync + Clone + 'static,
{
    fn default() -> Self {
        Self::new()
    }
}

impl<V> AsyncCache<V>
where
    V: Send + Sync + Clone + 'static,
{
    pub fn new() -> Self {
        Self {
            value: Arc::new(RwLock::new(None)),
        }
    }
    pub async fn get(&self) -> Option<V> {
        self.value.read().await.clone()
    }
    pub async fn insert(&self, value: V) -> Option<V> {
        self.value.write().await.replace(value)
    }
    pub async fn remove(&self) -> Option<V> {
        self.value.write().await.take()
    }
}
