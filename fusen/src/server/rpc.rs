use crate::protocol::fusen::{context::FusenContext, server::ServiceInfo};
use fusen_internal_common::BoxFuture;
use std::sync::Arc;

pub trait RpcServer: Send + Sync {
    fn invoke(&'static self, context: FusenContext) -> BoxFuture<FusenContext>;
    fn get_info(&self) -> Arc<ServiceInfo>;
}
