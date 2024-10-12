use std::collections::LinkedList;

use fusen_common::{error::BoxError, FusenContext};
use fusen_procedural_macro::Data;

use crate::FusenFuture;
pub mod server;

pub trait FusenFilter: Send + Sync + 'static {
    fn call(
        &'static self,
        join_point: ProceedingJoinPoint,
    ) -> FusenFuture<Result<FusenContext, crate::Error>>;
}

#[derive(Data)]
pub struct ProceedingJoinPoint {
    link: LinkedList<&'static dyn FusenFilter>,
    context: FusenContext,
}

impl ProceedingJoinPoint {
    pub fn new(link: LinkedList<&'static dyn FusenFilter>, context: FusenContext) -> Self {
        Self { link, context }
    }
    pub fn into_data(self) -> FusenContext {
        self.context
    }
    pub async fn proceed(mut self) -> Result<FusenContext, BoxError> {
        match self.link.pop_front() {
            Some(filter) => filter.call(self).await,
            None => Ok(self.into_data()),
        }
    }
}
