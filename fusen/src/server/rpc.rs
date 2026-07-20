use crate::{
    error::FusenError,
    filter::{FusenFilter, ProceedingJoinPoint},
    protocol::fusen::{context::FusenContext, service::ServiceInfo},
};
use std::{collections::HashMap, sync::Arc};

pub trait RpcService: Send + Sync + FusenFilter {
    fn get_service_info(&self) -> ServiceInfo;
}

#[derive(Clone, Default)]
pub struct RpcServerHandler {
    cache: HashMap<String, Arc<RpcServiceFilter>>,
}

struct RpcServiceFilter(Box<dyn RpcService>);

impl FusenFilter for RpcServiceFilter {
    fn call<'a>(
        &'a self,
        join_point: ProceedingJoinPoint,
    ) -> fusen_internal_common::BoxFutureV2<'a, Result<FusenContext, FusenError>> {
        self.0.call(join_point)
    }
}

impl RpcServerHandler {
    pub fn new(cache: HashMap<String, Box<dyn RpcService>>) -> Self {
        let mut leak_cache = HashMap::default();
        for (key, value) in cache {
            leak_cache.insert(key, Arc::new(RpcServiceFilter(value)));
        }
        Self { cache: leak_cache }
    }

    pub async fn call(
        &self,
        link: Arc<Vec<Arc<dyn FusenFilter>>>,
        context: FusenContext,
    ) -> Result<FusenContext, FusenError> {
        let service = self
            .cache
            .get(context.method_info.service_desc.get_tag())
            .cloned();
        match service {
            Some(service) => {
                let base_filter: Arc<dyn FusenFilter> = service;
                let join_point = ProceedingJoinPoint::new(link, base_filter, context);
                join_point.proceed().await
            }
            None => Err(FusenError::RouteNotFound(
                context.method_info.service_desc.get_tag().to_owned(),
            )),
        }
    }
}
