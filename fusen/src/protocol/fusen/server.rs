use fusen_internal_common::resource::service::MethodResource;

#[derive(Debug)]
pub struct ServiceInfo {
    pub id: String,
    pub version: Option<String>,
    pub group: Option<String>,
    pub methods: Vec<MethodResource>,
}

impl ServiceInfo {
    pub fn new(
        id: &str,
        version: Option<&str>,
        group: Option<&str>,
        methods: Vec<MethodResource>,
    ) -> ServiceInfo {
        Self {
            id: id.to_owned(),
            version: version.map(|e| e.to_owned()),
            group: group.map(|e| e.to_owned()),
            methods,
        }
    }
}
