use std::collections::HashMap;

use bytes::Bytes;
use http_body_util::combinators::BoxBody;

use crate::protocol::fusen::path::Path;

#[derive(Debug)]
pub enum Request {
    HttpRequest(http::Request<BoxBody<Bytes, hyper::Error>>),
    FusenRequest(FusenRequest),
}

#[derive(Debug)]
pub struct FusenRequest {
    pub path: Path,
    pub headers: HashMap<String, String>,
    pub extensions: Option<HashMap<String, String>>,
    pub body: Bytes,
}