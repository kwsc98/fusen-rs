use std::collections::HashMap;

#[derive(Debug, Clone)]
pub struct ServiceResource {
    pub service_id: String,
    pub group: Option<String>,
    pub version: Option<String>,
    pub methods: Vec<MethodResource>,
    pub addr: String,
    pub weight: Option<f64>,
    pub metadata: HashMap<String, String>,
}

#[derive(Clone, Debug)]
pub struct MethodResource {
    pub method_name: String,
    pub path: String,
    pub method: String,
}
