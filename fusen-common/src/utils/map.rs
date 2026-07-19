use std::{collections::HashMap, hash::Hash, sync::Arc};
use tokio::sync::RwLock;

#[derive(Clone)]
pub struct AsyncMap<K, V> {
    values: Arc<RwLock<HashMap<K, V>>>,
}

impl<K, V> Default for AsyncMap<K, V>
where
    K: Hash + Eq + Send + Sync + Clone + 'static,
    V: Send + Sync + Clone + 'static,
{
    fn default() -> Self {
        Self::new()
    }
}

impl<K, V> AsyncMap<K, V>
where
    K: Hash + Eq + Send + Sync + Clone + 'static,
    V: Send + Sync + Clone + 'static,
{
    pub fn new() -> Self {
        Self {
            values: Arc::new(RwLock::new(HashMap::new())),
        }
    }
    pub async fn get(&self, key: K) -> Option<V> {
        self.values.read().await.get(&key).cloned()
    }
    pub async fn insert(&self, key: K, value: V) -> Option<V> {
        self.values.write().await.insert(key, value)
    }
    pub async fn remove(&self, key: K) -> Option<V> {
        self.values.write().await.remove(&key)
    }
    pub async fn map_clone(&self) -> HashMap<K, V> {
        self.values.read().await.clone()
    }
}
