use self::loadbalance::{DefaultLoadBalance, LoadBalance_};
use std::{collections::HashMap, sync::Arc};
pub mod loadbalance;

#[derive(Clone)]
pub struct HandlerContext {
    context: HashMap<String, Arc<Handler>>,
    cache: HashMap<String, Arc<HandlerController>>,
}

impl Default for HandlerContext {
    fn default() -> Self {
        let mut context = Self {
            context: Default::default(),
            cache: Default::default(),
        };
        let handler = Handler::new(
            "DefaultLoadBalance".to_string(),
            HandlerInvoker::LoadBalance(Box::leak(Box::new(DefaultLoadBalance))),
        );
        context.insert(handler);
        context
            .load_controller(HandlerInfo::new(
                "DefaultFusenClientHandlerInfo".to_string(),
                vec![],
            ))
            .unwrap();
        context
    }
}

impl HandlerContext {
    pub fn insert(&mut self, handler: Handler) {
        self.context.insert(handler.id.clone(), Arc::new(handler));
    }

    fn get_handler(&self, key: &str) -> Option<Arc<Handler>> {
        self.context.get(key).cloned()
    }

    pub fn get_controller(&self, class_name: &str) -> Option<Arc<HandlerController>> {
        self.cache
            .get(class_name)
            .map_or(self.cache.get("DefaultFusenClientHandlerInfo"), Some)
            .cloned()
    }

    pub fn load_controller(&mut self, handler_info: HandlerInfo) -> Result<(), crate::Error> {
        let mut load_balance: Option<&'static dyn LoadBalance_> = None;
        for item in &handler_info.handlers_id {
            if let Some(handler) = self.get_handler(item) {
                match handler.handler_invoker {
                    HandlerInvoker::LoadBalance(handler) => load_balance.insert(handler),
                };
            }
        }
        if load_balance.is_none() {
            if let Some(handler) = self.get_handler("DefaultLoadBalance") {
                match handler.handler_invoker {
                    HandlerInvoker::LoadBalance(handler) => load_balance.insert(handler),
                };
            }
        }
        let handler_controller = HandlerController {
            load_balance: load_balance
                .ok_or_else(|| crate::Error::from("not find load_balance"))?,
        };
        self.cache
            .insert(handler_info.id, Arc::new(handler_controller));
        Ok(())
    }
}

pub struct HandlerController {
    load_balance: &'static dyn LoadBalance_,
}

impl HandlerController {
    pub fn get_load_balance(&self) -> &'static dyn LoadBalance_ {
        self.load_balance
    }
}

pub enum HandlerInvoker {
    LoadBalance(&'static dyn LoadBalance_),
}

pub struct HandlerInfo {
    id: String,
    handlers_id: Vec<String>,
}

impl HandlerInfo {
    pub fn new(id: String, handlers_id: Vec<String>) -> Self {
        HandlerInfo { id, handlers_id }
    }
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
