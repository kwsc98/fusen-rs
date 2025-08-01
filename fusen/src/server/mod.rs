use std::{collections::HashMap, sync::Arc};

use fusen_register::Register;

use crate::{error::FusenError, handler::HandlerContext, server::rpc::RpcService};

pub mod path;
pub mod router;
pub mod rpc;

pub struct FusenServerContext {
    port: u16,
    registers: Vec<Box<dyn Register>>,
    handler_context: Arc<HandlerContext>,
    services: HashMap<String, &'static dyn RpcService>,
}

impl FusenServerContext {
    pub async fn run(mut self) -> Result<(), FusenError> {
        let port = self.port;
        Ok(())
    }
}
