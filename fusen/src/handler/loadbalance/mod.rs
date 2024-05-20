use crate::protocol::socket::InvokerAssets;
use fusen_common::FusenFuture;
use rand::prelude::SliceRandom;
use std::sync::Arc;

#[allow(async_fn_in_trait)]
pub trait LoadBalance {
    async fn select(
        &self,
        invokers: Vec<Arc<InvokerAssets>>,
    ) -> Result<Arc<InvokerAssets>, crate::Error>;
}

pub trait LoadBalance_: Send + Sync {
    fn select_(
        &'static self,
        invokers: Vec<Arc<InvokerAssets>>,
    ) -> FusenFuture<Result<Arc<InvokerAssets>, crate::Error>>;
}

pub struct DefaultLoadBalance;

impl LoadBalance_ for DefaultLoadBalance {
    fn select_(
        &self,
        invokers: Vec<Arc<InvokerAssets>>,
    ) -> FusenFuture<Result<Arc<InvokerAssets>, crate::Error>> {
        Box::pin(async move {
            invokers
                .choose(&mut rand::thread_rng())
                .ok_or(crate::Error::from("not find server"))
                .cloned()
        })
    }
}
