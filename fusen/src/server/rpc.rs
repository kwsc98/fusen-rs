use crate::{
    error::FusenError,
    protocol::fusen::{context::FusenContext, service::ServiceInfo},
};
use fusen_internal_common::BoxFuture;
use std::collections::HashMap;

pub trait RpcService: Send + Sync {
    fn invoke(&'static self, context: FusenContext) -> BoxFuture<Result<FusenContext, FusenError>>;
    fn get_service_info(&self) -> ServiceInfo;
}

#[derive(Clone, Default)]
pub struct RpcServerHandler {
    cache: HashMap<String, &'static dyn RpcService>,
}

impl RpcServerHandler {
    pub async fn new(cache: HashMap<String, &'static dyn RpcService>) -> Self {
        Self { cache }
    }

    pub async fn call(
        &self,
        join_point: crate::filter::ProceedingJoinPoint,
    ) -> Result<FusenContext, FusenError> {
        let service = self
            .cache
            .get(&join_point.context.method_info.service_desc.service_id)
            .cloned();
        match service {
            Some(service) => service.invoke(join_point.proceed().await?).await,
            None => Err(FusenError::Impossible),
        }
    }
}
