use std::collections::HashMap;
use bytes::Bytes;

#[derive(Debug)]
pub struct FusenResponse {
    pub status: u16,
    pub querys: HashMap<String, String>,
    pub headers: HashMap<String, String>,
    pub extensions: Option<HashMap<String, String>>,
    pub body: Bytes,
}
