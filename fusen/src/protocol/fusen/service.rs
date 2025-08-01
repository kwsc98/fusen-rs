use std::str::FromStr;

use http::Method;

#[derive(Debug, Clone)]
pub struct ServiceDesc {
    pub service_id: String,
    pub version: Option<String>,
    pub group: Option<String>,
}

impl ServiceDesc {
    pub fn new(service_id: &str, version: Option<&str>, group: Option<&str>) -> Self {
        Self {
            service_id: service_id.to_owned(),
            version: version.map(|e| e.to_owned()),
            group: group.map(|e| e.to_owned()),
        }
    }
}

#[derive(Debug)]
pub struct MethodInfo {
    pub service_desc: ServiceDesc,
    pub method: Method,
    pub path: String,
    pub method_name: String,
    pub fields: Vec<(String, String)>,
}

impl MethodInfo {
    pub fn new(
        service_desc: ServiceDesc,
        method: String,
        path: String,
        method_name: String,
        fields: Vec<(String, String)>,
    ) -> Self {
        Self {
            service_desc,
            method: Method::from_str(&method).unwrap(),
            path,
            method_name,
            fields,
        }
    }
}

#[derive(Debug)]
pub struct ServiceInfo {
    pub service_desc: ServiceDesc,
    pub method_infos: Vec<MethodInfo>,
}

impl ServiceInfo {
    pub fn new(service_desc: ServiceDesc, method_infos: Vec<MethodInfo>) -> Self {
        Self {
            service_desc,
            method_infos,
        }
    }
}
