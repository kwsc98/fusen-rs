use serde_json::Value;
use std::collections::HashMap;

use crate::protocol::Protocol;

#[derive(Debug)]
pub struct FusenResponse {
    pub protocol: Protocol,
    pub status: u16,
    pub headers: HashMap<String, String>,
    pub extensions: Option<HashMap<String, String>>,
    pub bodys: Option<Value>,
}

impl Default for FusenResponse {
    fn default() -> Self {
        Self {
            protocol: Protocol::Fusen,
            status: 200,
            headers: Default::default(),
            extensions: Default::default(),
            bodys: Default::default(),
        }
    }
}
