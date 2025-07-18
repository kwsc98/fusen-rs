use crate::{
    protocol::fusen::{metadata::MetaData, path::Path, request::FusenRequest},
    server::ServerType,
};

#[derive(Debug)]
pub struct FusenContext {
    unique_identifier: String,
    server_type: ServerType,
    meta_data: MetaData,
    context_info: ContextInfo,
    request: FusenRequest,
    response: FusenResponse,
}

#[derive(Debug)]
pub struct ContextInfo {
    path: Path,
    class_name: String,
    method_name: String,
    version: Option<String>,
    group: Option<String>,
}
