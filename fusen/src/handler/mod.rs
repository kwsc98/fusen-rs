use crate::{
    filter::FusenFilter,
    handler::{
        aspect::DefaultAspect,
        loadbalance::{DefaultLoadBalance, LoadBalance_},
    },
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
        context
    }
}
