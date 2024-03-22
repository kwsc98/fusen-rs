use super::FusenFilter;
use fusen_common::{error::FusenError, server::RpcServer, FusenContext};
use std::{collections::HashMap, sync::Arc};

#[derive(Clone, Default)]
pub struct RpcServerFilter {
    cache: HashMap<String, Arc<Box<dyn RpcServer>>>,
    path_cache: HashMap<String, (String, String)>,
}

impl RpcServerFilter {
    pub fn new(cache: HashMap<String, Arc<Box<dyn RpcServer>>>) -> Self {
        let mut path_cache = HashMap::new();
        for item in &cache {
            let (id, _version, methods) = item.1.get_info();
            for method in methods {
                let method_info = method.into();
                let path_rpc = "/".to_owned() + id + "/" + &method_info.0;
                let path = method_info.1;
                path_cache.insert(path_rpc, (id.to_string(), method_info.2.clone()));
                path_cache.insert(path, (id.to_string(), method_info.2));
            }
        }
        return RpcServerFilter { cache, path_cache };
    }
    pub fn get_server(&self, msg: &mut FusenContext) -> Option<Arc<Box<dyn RpcServer>>> {
        let info = self.path_cache.get(&msg.path)?;
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
                msg.res = Err(FusenError::ResourceEmpty(format!(
                    "not find server by {:?} version {:?}",
                    msg.class_name, msg.version
                )));
                Ok(msg)
            }),
        }
    }
}
