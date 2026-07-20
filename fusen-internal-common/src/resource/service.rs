use std::collections::HashMap;

/// The HTTP location from which one generated RPC argument is read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParameterSource {
    /// A named `{parameter}` segment in the route template.
    Path,
    /// A URL query parameter.
    Query,
    /// A JSON request body argument.
    Body,
}

/// Wire-level metadata for one generated RPC argument.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParameterResource {
    pub name: String,
    pub source: ParameterSource,
}

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
    pub parameters: Vec<ParameterResource>,
}
