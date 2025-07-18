use http::Uri;

#[derive(Debug, Clone)]
pub struct Path {
    pub method: Method,
    pub uri: Uri,
}

#[derive(Debug, Clone)]
pub enum Method {
    GET,
    POST,
    DELETE,
    PUT,
}
