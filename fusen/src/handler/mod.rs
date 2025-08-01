use crate::{
    error::FusenError,
    filter::FusenFilter,
    handler::{
        aspect::DefaultAspect,
        loadbalance::{DefaultLoadBalance, LoadBalance_},
    },
    protocol::fusen::service::ServiceDesc,
};
use std::{collections::HashMap, sync::Arc};

pub mod aspect;
pub mod loadbalance;

#[derive(Clone)]
pub struct HandlerController {
    pub load_balance: &'static dyn LoadBalance_,
    pub aspect: Arc<Vec<&'static dyn FusenFilter>>,
}

pub enum HandlerInvoker {
    LoadBalance(&'static dyn LoadBalance_),
    Aspect(&'static dyn FusenFilter),
}

pub struct HandlerContext {
    handlers: HashMap<String, Arc<HandlerInvoker>>,
    cache: HashMap<String, HandlerController>,
}

pub struct HandlerInfo {
    service_desc: ServiceDesc,
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
        let _ = context.load_controller(HandlerInfo {
            service_desc: ServiceDesc {
                service_id: "DefaultHandlerController".to_string(),
                version: Default::default(),
                group: Default::default(),
            },
            handlers: vec![],
        });
        context
    }
}

impl HandlerContext {
    pub fn get_controller(&self, service_desc: &ServiceDesc) -> &HandlerController {
        self.cache
            .get(&service_desc.service_id)
            .unwrap_or(self.cache.get("DefaultHandlerController").unwrap())
    }
    pub fn load_controller(&mut self, handler_info: HandlerInfo) -> Result<(), FusenError> {
        let mut load_balance: Option<&'static dyn LoadBalance_> = None;
        let mut aspect: Vec<&'static dyn FusenFilter> = Vec::new();
        for handler_id in &handler_info.handlers {
            if let Some(handler_invoker) = self.get_handler(handler_id) {
                match handler_invoker.as_ref() {
                    HandlerInvoker::LoadBalance(handler) => {
                        let _ = load_balance.insert(*handler);
                    }
                    HandlerInvoker::Aspect(handler) => {
                        aspect.push(*handler);
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
            aspect: Arc::new(aspect),
        };
        self.cache
            .insert(handler_info.service_desc.service_id, handler_controller);
        Ok(())
    }
    fn get_handler(&self, handler_id: &str) -> Option<Arc<HandlerInvoker>> {
        self.handlers.get(handler_id).cloned()
    }
}
