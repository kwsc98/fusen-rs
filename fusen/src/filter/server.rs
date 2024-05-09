use super::FusenFilter;
use fusen_common::{error::FusenError, server::RpcServer, FusenContext, MethodResource, Path};
use std::{collections::HashMap, sync::Arc};

#[derive(Clone, Default)]
pub struct RpcServerFilter {
    cache: HashMap<String, Arc<&'static dyn RpcServer>>,
    path_cache: HashMap<String, (String, String)>,
}

impl RpcServerFilter {
    pub fn new(cache: HashMap<String, Arc<&'static dyn RpcServer>>) -> Self {
        let mut path_cache = HashMap::new();
        for item in &cache {
            let info = item.1.get_info();
            for method in info.methods {
                let MethodResource {
                    id,
                    path,
                    name,
                    method,
                } = method;
                let path_rpc = "/".to_owned() + &info.id + "/" + &id;
                path_cache.insert(
                    Path::POST(path_rpc).get_key(),
                    (info.id.to_string(), name.clone()),
                );
                path_cache.insert(
                    Path::new(&method, path).get_key(),
                    (info.id.to_string(), name),
                );
            }
        }
        return RpcServerFilter { cache, path_cache };
    }
    pub fn get_server(&self, msg: &mut FusenContext) -> Option<Arc<&'static dyn RpcServer>> {
        let info = self.path_cache.get(&msg.path.get_key())?;
        msg.class_name = info.0.clone();
        msg.method_name = info.1.clone();
        let mut class_name = msg.class_name.clone();
        if let Some(version) = &msg.version {
            class_name.push_str(":");
            class_name.push_str(version);
        }
        self.cache.get(&class_name).map(|e| e.clone())
    }
}

impl FusenFilter for RpcServerFilter {
    type Request = FusenContext;

    type Response = FusenContext;

    type Error = FusenError;

    type Future = crate::FusenFuture<Result<Self::Response, Self::Error>>;

    fn call(&self, req: Self::Request) -> Self::Future {
        let mut msg: FusenContext = req;
        let server = self.get_server(&mut msg);
        match server {
            Some(server) => Box::pin(async move { Ok(server.invoke(msg).await) }),
            None => Box::pin(async move {
                msg.res = Err(FusenError::NotFind(format!(
                    "not find server by {:?} version {:?}",
                    msg.class_name, msg.version
                )));
                Ok(msg)
            }),
        }
    }
}
