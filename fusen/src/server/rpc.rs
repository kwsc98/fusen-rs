use crate::protocol::fusen::{context::FusenContext, service::ServiceInfo};
use fusen_internal_common::BoxFuture;
use std::{collections::HashMap, sync::Arc};

pub trait RpcServer: Send + Sync {
    fn invoke(&'static self, context: FusenContext) -> BoxFuture<FusenContext>;
    fn get_info(&self) -> Arc<ServiceInfo>;
}

#[derive(Clone, Default)]
pub struct RpcServerFilter {
    cache: HashMap<String, &'static dyn RpcServer>,
}

