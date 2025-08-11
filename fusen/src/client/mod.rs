use crate::{
    error::FusenError,
    handler::{Handler, HandlerContext, HandlerInfo},
    protocol::Protocol,
};
use fusen_register::Register;
use serde_json::Value;
use std::sync::Arc;

pub struct FusenClientContextBuilder {
    register: Option<Box<dyn Register>>,
    handler_context: HandlerContext,
    service_handlers: Vec<HandlerInfo>,
}

impl FusenClientContextBuilder {
    pub fn new() -> Self {
        Self {
            register: Default::default(),
            handler_context: HandlerContext::default(),
            service_handlers: Default::default(),
        }
    }

    pub fn builder(self) -> FusenClientContext {
        let mut handler_context = self.handler_context;
        for handler_info in self.service_handlers {
            handler_context.load_controller(handler_info)
        }
        FusenClientContext {
            register: self.register.map(|e| Arc::new(e)),
            handler_context: Arc::new(handler_context),
        }
    }

    pub fn register(mut self, register: Box<dyn Register>) -> Self {
        self.register.insert(register);
        self
    }

    pub fn handler(mut self, handler: Handler) -> Self {
        self.handler_context.load_handler(handler);
        self
    }
}

pub struct FusenClientContext {
    register: Option<Arc<Box<dyn Register>>>,
    handler_context: Arc<HandlerContext>,
}

impl FusenClientContext {
    pub fn protocol(&self, protocol: Protocol) -> FusenClient {
        FusenClient {
            protocol,
            register: self.register.clone(),
            handler_context: self.handler_context.clone(),
        }
    }
}

pub struct FusenClient {
    pub protocol: Protocol,
    pub register: Option<Arc<Box<dyn Register>>>,
    pub handler_context: Arc<HandlerContext>,
}

impl FusenClient {
    pub async fn invoke(
        &self,
        method: &str,
        url: &str,
        field_pats: &[&str],
        request_body: Vec<Value>,
    ) -> Result<Value, FusenError> {
        todo!()
    }
}
