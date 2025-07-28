use http::{Method, Uri};
use serde_json::Value;
use std::collections::HashMap;

use crate::protocol::Protocol;

#[derive(Debug)]
pub struct FusenRequest {
    pub protocol: Protocol,
    pub path: Path,
    pub querys: HashMap<String, String>,
    pub headers: HashMap<String, String>,
    pub extensions: Option<HashMap<String, String>>,
    pub bodys: Option<Vec<Value>>,
}

#[derive(Debug)]
pub struct Path {
    pub method: Method,
    pub uri: Uri,
}
