use std::str::FromStr;

use http::Method;

#[derive(Debug, Clone)]
pub struct ServiceDesc {
    pub service_id: String,
    pub version: Option<String>,
    pub group: Option<String>,
    tag: String,
}

impl ServiceDesc {
    pub fn new(service_id: &str, version: Option<&str>, group: Option<&str>) -> Self {
        let tag = format!("{}:{:?}:{:?}", service_id, version, group);
        Self {
            service_id: service_id.to_owned(),
            version: version.map(|e| e.to_owned()),
            group: group.map(|e| e.to_owned()),
            tag,
        }
    }

    pub fn get_tag(&self) -> &str {
        &self.tag
    }
}

#[derive(Debug, Clone)]
pub struct MethodInfo {
    pub service_desc: ServiceDesc,
    pub method_name: String,
    pub method: Method,
    pub path: String,
    pub fields: Vec<(String, String)>,
}

impl MethodInfo {
    pub fn new(
        service_desc: ServiceDesc,
        method_name: String,
        method: String,
        path: String,
        fields: Vec<(String, String)>,
    ) -> Self {
        Self {
            service_desc,
            method_name,
            method: Method::from_str(&method).unwrap(),
            path,
            fields,
        }
    }
}

#[derive(Debug, Clone)]
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
