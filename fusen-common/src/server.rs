use crate::{FusenContext, FusenFuture, MethodResource};

#[derive(Clone)]
pub enum Protocol {
    HTTP(String),
    HTTP2(String),
}

pub trait RpcServer: Send + Sync {
    fn invoke(&self, msg: FusenContext) -> FusenFuture<FusenContext>;
    fn get_info(&self) -> (&str, Option<&str>, Vec<MethodResource>);
}
