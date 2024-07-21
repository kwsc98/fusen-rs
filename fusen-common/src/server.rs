use crate::{FusenContext, FusenFuture, MethodResource};

pub trait RpcServer: Send + Sync {
    fn invoke(&'static self, msg: FusenContext) -> FusenFuture<FusenContext>;
    fn get_info(&self) -> ServerInfo;
}

#[derive(Debug)]
pub struct ServerInfo {
    pub id: String,
    pub version: Option<String>,
    pub group: Option<String>,
    pub methods: Vec<MethodResource>,
}

#[derive(Debug)]
pub struct ClientInfo {
    pub id: String,
    pub version: Option<String>,
    pub group: Option<String>,
    pub methods: Vec<MethodResource>,
}

impl ServerInfo {
    pub fn new(
        id: &str,
        version: Option<&str>,
        group: Option<&str>,
        methods: Vec<MethodResource>,
    ) -> ServerInfo {
        Self {
            id: id.to_owned(),
            version: version.map(|e| e.to_owned()),
            group: group.map(|e| e.to_owned()),
            methods,
        }
    }
}

impl ClientInfo {
    pub fn new(
        id: &str,
        version: Option<&str>,
        group: Option<&str>,
        methods: Vec<MethodResource>,
    ) -> ClientInfo {
        Self {
            id: id.to_owned(),
            version: version.map(|e| e.to_owned()),
            group: group.map(|e| e.to_owned()),
            methods,
        }
    }
}
