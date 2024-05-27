use fusen_common::FusenContext;

use crate::FusenFuture;
pub mod server;

pub trait FusenFilter : Send + Sync + 'static {
    fn call(&'static self, context: FusenContext) -> FusenFuture<Result<FusenContext, crate::Error>>;
}