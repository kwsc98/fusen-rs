use crate::{
    error::FusenError,
    filter::FusenFilter,
    handler::{
        aspect::DefaultAspect,
        loadbalance::{DefaultLoadBalance, LoadBalance_},
    },
    protocol::fusen::service::ServiceInfo,
};
use std::{
    collections::{HashMap, LinkedList},
    sync::Arc,
};

pub mod aspect;
pub mod loadbalance;

pub struct HandlerController {
    load_balance: &'static dyn LoadBalance_,
    aspect: LinkedList<&'static dyn FusenFilter>,
}

pub enum HandlerInvoker {
    LoadBalance(&'static dyn LoadBalance_),
    Aspect(&'static dyn FusenFilter),
}

pub struct HandlerContext {
    handlers: HashMap<String, Arc<HandlerInvoker>>,
    cache: HashMap<String, Arc<HandlerController>>,
}

pub struct HandlerInfo {
    service_info: ServiceInfo,
    handlers: Vec<String>,
}

impl Default for HandlerContext {
    fn default() -> Self {
        let mut context = Self {
            handlers: Default::default(),
            cache: Default::default(),
        };
        context.handlers.insert(
            "DefaultLoadBalance".to_string(),
            Arc::new(HandlerInvoker::LoadBalance(Box::leak(Box::new(
                DefaultLoadBalance,
            )))),
        );
        context.handlers.insert(
            "DefaultAspect".to_string(),
            Arc::new(HandlerInvoker::Aspect(Box::leak(Box::new(DefaultAspect)))),
        );
        context.load_controller(HandlerInfo {
            service_info: ServiceInfo {
                server_id: "DefaultHandlerController".to_string(),
                service_name: Default::default(),
                version: Default::default(),
                group: Default::default(),
            },
            handlers: vec![],
        });
        context
    }
}

impl HandlerContext {
    pub fn get_controller(&self, service_info: &ServiceInfo) -> &HandlerController {
        self.cache
            .get(&service_info.server_id)
            .unwrap_or(self.cache.get("DefaultHandlerController").unwrap())
    }
    pub fn load_controller(&mut self, handler_info: HandlerInfo) -> Result<(), FusenError> {
        let mut load_balance: Option<&'static dyn LoadBalance_> = None;
        let mut aspect: LinkedList<&'static dyn FusenFilter> = LinkedList::new();

        for handler_id in &handler_info.handlers {
            if let Some(handler_invoker) = self.get_handler(handler_id) {
                match handler_invoker.as_ref() {
                    HandlerInvoker::LoadBalance(handler) => {
                        let _ = load_balance.insert(*handler);
                    }
                    HandlerInvoker::Aspect(handler) => {
                        aspect.push_back(*handler);
                    }
                };
            }
        }
        if load_balance.is_none() {
            if let Some(handler_invoker) = self.get_handler("DefaultLoadBalance") {
                match handler_invoker.as_ref() {
                    HandlerInvoker::LoadBalance(handler) => load_balance.insert(*handler),
                    _ => return Err(FusenError::Impossible),
                };
            }
        }
        let handler_controller = HandlerController {
            load_balance: load_balance.unwrap(),
            aspect,
        };
        self.cache.insert(
            handler_info.service_info.server_id,
            Arc::new(handler_controller),
        );
        Ok(())
    }
    fn get_handler(&self, handler_id: &str) -> Option<Arc<HandlerInvoker>> {
        self.handlers.get(handler_id).cloned()
    }
}
