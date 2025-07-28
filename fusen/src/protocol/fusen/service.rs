use http::Method;

#[derive(Debug)]
pub struct ServiceInfo {
    pub service_name: String,
    pub method_name: String,
    pub version: Option<String>,
    pub group: Option<String>,
}

#[derive(Debug)]
pub struct MethodInfo {
    pub service_info: ServiceInfo,
    pub method: Method,
    pub path: String,
    pub method_name: String,
    pub fields: Vec<(String, String)>,
}

