use aspect::{Aspect_, DefaultAspect};
use serde::{Deserialize, Serialize};

use self::loadbalance::{DefaultLoadBalance, LoadBalance_};
use std::{collections::HashMap, sync::Arc};
pub mod aspect;
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
        let aspect = Handler::new(
            "DefaultAspect".to_string(),
            HandlerInvoker::Aspect(Box::leak(Box::new(DefaultAspect))),
        );
        context.insert(handler);
        context.insert(aspect);
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

    pub fn get_controller(&self, class_name: &str) -> &Arc<HandlerController> {
        self.cache.get(class_name).map_or(
            self.cache.get("DefaultFusenClientHandlerInfo").unwrap(),
            |e| e,
        )
    }

    pub fn insert_controller(&mut self, key: String, controller: Arc<HandlerController>) {
        self.cache.insert(key, controller);
    }

    pub fn load_controller(&mut self, handler_info: HandlerInfo) -> Result<(), crate::Error> {
        let mut load_balance: Option<&'static dyn LoadBalance_> = None;
        let mut aspect: Option<&'static dyn Aspect_> = None;

        for item in &handler_info.handlers_id {
            if let Some(handler) = self.get_handler(item) {
                match handler.handler_invoker {
                    HandlerInvoker::LoadBalance(handler) => {
                        let _ = load_balance.insert(handler);
                    }
                    HandlerInvoker::Aspect(handler) => {
                        let _ = aspect.insert(handler);
                    }
                };
            }
        }
        if load_balance.is_none() {
            if let Some(handler) = self.get_handler("DefaultLoadBalance") {
                match handler.handler_invoker {
                    HandlerInvoker::LoadBalance(handler) => load_balance.insert(handler),
                    _ => return Err(crate::Error::from("DefaultLoadBalance get Error")),
                };
            }
        }
        if aspect.is_none() {
            if let Some(handler) = self.get_handler("DefaultAspect") {
                match handler.handler_invoker {
                    HandlerInvoker::Aspect(handler) => aspect.insert(handler),
                    _ => return Err(crate::Error::from("DefaultAspect get Error")),
                };
            }
        }
        let handler_controller = HandlerController {
            load_balance: load_balance
                .ok_or_else(|| crate::Error::from("not find load_balance"))?,
            aspect: aspect.ok_or_else(|| crate::Error::from("not find aspect"))?,
        };
        self.cache
            .insert(handler_info.id, Arc::new(handler_controller));
        Ok(())
    }
}

pub struct HandlerController {
    load_balance: &'static dyn LoadBalance_,
    aspect: &'static dyn Aspect_,
}

impl HandlerController {
    pub fn get_load_balance(&self) -> &'static dyn LoadBalance_ {
        self.load_balance
    }
    pub fn get_aspect(&self) -> &'static dyn Aspect_ {
        self.aspect
    }
}

pub enum HandlerInvoker {
    LoadBalance(&'static dyn LoadBalance_),
    Aspect(&'static dyn Aspect_),
}

#[derive(Serialize, Deserialize)]
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
