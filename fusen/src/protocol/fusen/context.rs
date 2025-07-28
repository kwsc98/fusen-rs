use crate::protocol::fusen::{
    metadata::MetaData, request::FusenRequest, response::FusenResponse, service::MethodInfo,
};
use std::sync::Arc;

#[derive(Debug)]
pub struct FusenContext {
    pub unique_identifier: String,
    pub metadata: MetaData,
    pub method_info: Arc<MethodInfo>,
    pub request: FusenRequest,
    pub response: FusenResponse,
}
