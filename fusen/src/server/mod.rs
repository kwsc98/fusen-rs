use std::{collections::HashMap, sync::Arc};

use fusen_register::Register;

use crate::{error::FusenError, handler::HandlerContext, server::rpc::RpcServer};

pub mod rpc;
pub mod router;

pub struct FusenServerContext {
    port: u16,
    registers: Vec<Box<dyn Register>>,
    handler_context: Arc<HandlerContext>,
    services: HashMap<String, &'static dyn RpcServer>,
}

impl FusenServerContext {
    pub async fn run(mut self) -> Result<(), FusenError> {
        let port = self.port;

        Ok(())
    }
}
