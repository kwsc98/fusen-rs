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
    pub load_balance: Arc<Box<dyn LoadBalance_>>,
    pub aspect: Arc<Vec<Arc<Box<dyn FusenFilter>>>>,
}

pub enum HandlerInvoker {
    LoadBalance(Arc<Box<dyn LoadBalance_>>),
    Aspect(Arc<Box<dyn FusenFilter>>),
}

pub struct HandlerContext {
    handlers: HashMap<String, Arc<HandlerInvoker>>,
    cache: HashMap<String, HandlerController>,
}

pub struct HandlerInfo {
    pub service_desc: ServiceDesc,
    pub handlers: Vec<String>,
}

impl Default for HandlerContext {
    fn default() -> Self {
        let mut context = Self {
            handlers: Default::default(),
            cache: Default::default(),
        };
        context.handlers.insert(
            "DefaultLoadBalance".to_string(),
            Arc::new(HandlerInvoker::LoadBalance(Arc::new(Box::new(
                DefaultLoadBalance,
            )))),
        );
        context.handlers.insert(
            "DefaultAspect".to_string(),
            Arc::new(HandlerInvoker::Aspect(Arc::new(Box::new(DefaultAspect)))),
        );
        context.load_controller(HandlerInfo {
            service_desc: ServiceDesc::new("DefaultHandlerController", None, None),
            handlers: vec![],
        });
        context
    }
}

impl HandlerContext {
    pub fn load_handler(&mut self, handler: Handler) {
        self.handlers
            .insert(handler.id, Arc::new(handler.handler_invoker));
    }

    pub fn get_controller(&self, service_desc: &ServiceDesc) -> &HandlerController {
        self.cache.get(service_desc.get_tag()).unwrap_or(
            self.cache
                .get("DefaultHandlerController:None:None")
                .unwrap(),
        )
    }

    pub fn load_controller(&mut self, handler_info: HandlerInfo) {
        let mut load_balance: Option<Arc<Box<dyn LoadBalance_>>> = None;
        let mut aspect: Vec<Arc<Box<dyn FusenFilter>>> = Vec::new();
        for handler_id in &handler_info.handlers {
            if let Some(handler_invoker) = self.get_handler(handler_id) {
                match handler_invoker.as_ref() {
                    HandlerInvoker::LoadBalance(handler) => {
                        let _ = load_balance.insert(handler.clone());
                    }
                    HandlerInvoker::Aspect(handler) => {
                        aspect.push(handler.clone());
                    }
                };
            }
        }
        if load_balance.is_none()
            && let Some(handler_invoker) = self.get_handler("DefaultLoadBalance")
        {
            match handler_invoker.as_ref() {
                HandlerInvoker::LoadBalance(handler) => load_balance.insert(handler.clone()),
                _ => panic!("{}", FusenError::Impossible),
            };
        }

        let handler_controller = HandlerController {
            load_balance: load_balance.unwrap(),
            aspect: Arc::new(aspect),
        };
        self.cache.insert(
            handler_info.service_desc.get_tag().to_owned(),
            handler_controller,
        );
    }

    fn get_handler(&self, handler_id: &str) -> Option<Arc<HandlerInvoker>> {
        self.handlers.get(handler_id).cloned()
    }
}

pub struct Handler {
    pub id: String,
    pub handler_invoker: HandlerInvoker,
}

pub trait HandlerLoad {
    fn load(self) -> Handler;
}
