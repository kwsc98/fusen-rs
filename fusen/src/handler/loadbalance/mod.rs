use crate::{protocol::socket::InvokerAssets, register::ResourceInfo};
use fusen_common::FusenFuture;
use std::sync::Arc;

#[allow(async_fn_in_trait)]
pub trait LoadBalance {
    async fn select(&self, invokers: Arc<ResourceInfo>)
        -> Result<Arc<InvokerAssets>, crate::Error>;
}

pub trait LoadBalance_: Send + Sync {
    fn select_(
        &'static self,
        invokers: Arc<ResourceInfo>,
    ) -> FusenFuture<Result<Arc<InvokerAssets>, crate::Error>>;
}

pub struct DefaultLoadBalance;

impl LoadBalance_ for DefaultLoadBalance {
    fn select_(
        &self,
        invokers: Arc<ResourceInfo>,
    ) -> FusenFuture<Result<Arc<InvokerAssets>, crate::Error>> {
        Box::pin(async move { invokers.select().ok_or("not find server".into()) })
    }
}
