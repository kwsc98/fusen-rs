use super::FusenFilter;
use fusen_common::{
    error::FusenError, server::RpcServer, FusenContext, FusenFuture, MethodResource, Path,
};
use std::collections::HashMap;

#[derive(Clone, Default)]
pub struct RpcServerFilter {
    cache: HashMap<String, &'static dyn RpcServer>,
    path_cache: HashMap<String, (String, String)>,
}

impl RpcServerFilter {
    pub fn new(cache: HashMap<String, &'static dyn RpcServer>) -> Self {
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
        RpcServerFilter { cache, path_cache }
    }
    pub fn get_path_cache(&self) -> HashMap<String, (String, String)> {
        self.path_cache.clone()
    }

    pub fn get_server(&self, context: &mut FusenContext) -> Option<&'static dyn RpcServer> {
        let context_info = &context.context_info;
        let mut class_name = context_info.class_name.clone();
        if let Some(version) = &context_info.version {
            class_name.push(':');
            class_name.push_str(version);
        }
        self.cache.get(&class_name).copied()
    }
}

impl FusenFilter for RpcServerFilter {
    fn call(&self, mut context: FusenContext) -> FusenFuture<Result<FusenContext, crate::Error>> {
        let server = self.get_server(&mut context);
        match server {
            Some(server) => Box::pin(async move { Ok(server.invoke(context).await) }),
            None => Box::pin(async move {
                context.response.response = Err(FusenError::NotFind);
                Ok(context)
            }),
        }
    }
}
