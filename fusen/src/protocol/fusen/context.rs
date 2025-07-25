use crate::protocol::{fusen::{metadata::MetaData, request::FusenRequest, response::FusenResponse}, Protocol};

#[derive(Debug)]
pub struct FusenContext {
    pub unique_identifier: String,
    pub metadata: MetaData,
    pub context_info: ContextInfo,
    pub protocol: Protocol,
    pub request: FusenRequest,
    pub response: FusenResponse,
}

#[derive(Debug)]
pub struct ContextInfo {
    pub class_name: String,
    pub method_name: String,
    pub version: Option<String>,
    pub group: Option<String>,
}
