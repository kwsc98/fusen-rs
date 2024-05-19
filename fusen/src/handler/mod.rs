use self::loadbalance::LoadBalance_;
use std::{collections::HashMap, sync::Arc};
pub mod loadbalance;

pub struct HandlerContext {
    cache: HashMap<String, Arc<Handler>>,
}

impl HandlerContext {
    pub fn insert(&mut self, handler: Handler) {
        let mut key = String::new();
        match &handler.handler_invoker {
            HandlerInvoker::LoadBalance(_) => key.push_str("LoadBalance:"),
        }
        key.push_str(&handler.id);
        self.cache.insert(key, Arc::new(handler));
    }
    pub fn get_load_balance(&self, key: &str) -> Option<&'static dyn LoadBalance_> {
        let mut cache_key: String = String::from("LoadBalance:");
        cache_key.push_str(key);
        let mut handler = self.cache.get(cache_key.as_str());
        if let None = handler {
            handler = self.cache.get("LoadBalance:default");
        }
        handler.map_or(None, |e| match e.handler_invoker {
            HandlerInvoker::LoadBalance(handler) => Some(handler),
        })
    }
}

pub enum HandlerInvoker {
    LoadBalance(&'static dyn LoadBalance_),
}

pub struct Handler {
    id: String,
    handler_invoker: HandlerInvoker,
}

impl Handler {
    pub fn new(id: String, handler_invoker: HandlerInvoker) -> Self {
        Self {
            id,
            handler_invoker,
        }
    }
}

pub trait HandlerLoad {
    fn load(self) -> Handler;
}
