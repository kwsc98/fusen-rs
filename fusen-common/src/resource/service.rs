use std::{collections::HashMap, net::SocketAddr};

#[derive(Debug, Clone)]
pub struct ServiceResource {
    pub service_name: String,
    pub group: Option<String>,
    pub version: Option<String>,
    pub methods: Vec<MethodResource>,
    pub socket_addr: SocketAddr,
    pub weight: Option<f64>,
    pub metadata: HashMap<String, String>,
}

#[derive(Clone, Debug)]
pub struct MethodResource {
    pub method_name: String,
    pub path: String,
    pub method: String,
}
