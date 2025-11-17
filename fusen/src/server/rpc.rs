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
    cache: HashMap<String, Arc<Box<dyn FusenFilter>>>,
}

impl RpcServerHandler {
    pub fn new(cache: HashMap<String, Box<dyn RpcService>>) -> Self {
        let mut leak_cache: HashMap<String, Arc<Box<dyn FusenFilter>>> = HashMap::default();
        for (key, value) in cache {
            let _ = leak_cache.insert(key, Arc::new(value));
        }
        Self { cache: leak_cache }
    }

    pub async fn call(
        &self,
        link: Arc<Vec<Arc<Box<dyn FusenFilter>>>>,
        context: FusenContext,
    ) -> Result<FusenContext, FusenError> {
        let service = self
            .cache
            .get(context.method_info.service_desc.get_tag())
            .cloned();
        match service {
            Some(service) => {
                let join_point = ProceedingJoinPoint::new(link, service, context);
                join_point.proceed().await
            }
            None => Err(FusenError::Impossible),
        }
    }
}
