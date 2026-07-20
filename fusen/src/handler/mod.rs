use crate::{
    error::FusenError,
    filter::FusenFilter,
    handler::{
        aspect::DefaultAspect,
        loadbalance::{DefaultLoadBalance, LoadBalanceDyn},
    },
    protocol::fusen::service::ServiceDesc,
};
use std::{collections::HashMap, sync::Arc};

pub mod aspect;
pub mod loadbalance;

#[derive(Clone)]
pub struct HandlerController {
    pub load_balance: Option<Arc<dyn LoadBalanceDyn>>,
    pub aspect: Arc<Vec<Arc<dyn FusenFilter>>>,
}

pub enum HandlerInvoker {
    LoadBalance(Arc<dyn LoadBalanceDyn>),
    Aspect(Arc<dyn FusenFilter>),
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
        let default_load_balance: Arc<dyn LoadBalanceDyn> = Arc::new(DefaultLoadBalance);
        let mut context = Self {
            handlers: Default::default(),
            cache: Default::default(),
        };
        context.handlers.insert(
            "DefaultLoadBalance".to_string(),
            Arc::new(HandlerInvoker::LoadBalance(default_load_balance.clone())),
        );
        context.handlers.insert(
            "DefaultAspect".to_string(),
            Arc::new(HandlerInvoker::Aspect(Arc::new(DefaultAspect))),
        );
        context.cache.insert(
            ServiceDesc::new("DefaultHandlerController", None, None)
                .get_tag()
                .to_owned(),
            HandlerController {
                load_balance: Some(default_load_balance),
                aspect: Arc::new(Vec::new()),
            },
        );
        context
    }
}

impl HandlerContext {
    pub fn load_handler(&mut self, handler: Handler) -> Result<(), FusenError> {
        if self.handlers.contains_key(&handler.id) {
            return Err(FusenError::InvalidRequest(format!(
                "duplicate handler id {}",
                handler.id
            )));
        }
        self.handlers
            .insert(handler.id, Arc::new(handler.handler_invoker));
        Ok(())
    }

    pub fn get_controller(
        &self,
        service_desc: &ServiceDesc,
    ) -> Result<&HandlerController, FusenError> {
        Ok(self.cache.get(service_desc.get_tag()).unwrap_or(
            self.cache
                .get("DefaultHandlerController:None:None")
                .ok_or_else(|| FusenError::Internal {
                    message: "default handler controller is missing",
                    source: Box::new(std::io::Error::other("invalid handler invariant")),
                })?,
        ))
    }

    pub fn load_controller(&mut self, handler_info: HandlerInfo) -> Result<(), FusenError> {
        let mut load_balance: Option<Arc<dyn LoadBalanceDyn>> = None;
        let mut aspect: Vec<Arc<dyn FusenFilter>> = Vec::new();
        for handler_id in &handler_info.handlers {
            let handler_invoker = self.get_handler(handler_id).ok_or_else(|| {
                FusenError::InvalidRequest(format!("unknown handler id {handler_id}"))
            })?;
            match handler_invoker.as_ref() {
                HandlerInvoker::LoadBalance(handler) => {
                    load_balance = Some(handler.clone());
                }
                HandlerInvoker::Aspect(handler) => {
                    aspect.push(handler.clone());
                }
            };
        }
        if load_balance.is_none()
            && let Some(handler_invoker) = self.get_handler("DefaultLoadBalance")
        {
            match handler_invoker.as_ref() {
                HandlerInvoker::LoadBalance(handler) => load_balance.insert(handler.clone()),
                _ => {
                    return Err(FusenError::Internal {
                        message: "default load balancer has an invalid type",
                        source: Box::new(std::io::Error::other("invalid handler invariant")),
                    });
                }
            };
        }

        let handler_controller = HandlerController {
            load_balance,
            aspect: Arc::new(aspect),
        };
        self.cache.insert(
            handler_info.service_desc.get_tag().to_owned(),
            handler_controller,
        );
        Ok(())
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
