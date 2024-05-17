use std::sync::Arc;
use fusen_common::FusenFuture;
use crate::protocol::socket::InvokerAssets;

#[allow(async_fn_in_trait)]
pub trait LoadBalance {
    async fn select(
        &self,
        invokers: Vec<Arc<InvokerAssets>>,
    ) -> Result<Arc<InvokerAssets>, crate::Error>;
}

pub trait LoadBalance_ {
    fn select_(
        &'static self,
        invokers: Vec<Arc<InvokerAssets>>,
    ) -> FusenFuture<Result<Arc<InvokerAssets>, crate::Error>>;
}
