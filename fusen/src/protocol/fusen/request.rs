use http::{Method, Uri};
use serde_json::Value;
use std::collections::HashMap;

#[derive(Debug)]
pub struct FusenRequest {
    pub path: Path,
    pub querys: HashMap<String, String>,
    pub headers: HashMap<String, String>,
    pub extensions: Option<HashMap<String, String>>,
    pub body: Vec<Value>,
}

#[derive(Debug)]
pub struct Path {
    pub method: Method,
    pub uri: Uri,
}
