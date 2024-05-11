use super::FusenFilter;
use fusen_common::{error::FusenError, server::RpcServer, FusenContext, MethodResource, Path};
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
    pub fn get_server(&self, context: &mut FusenContext) -> Option<&'static dyn RpcServer> {
        let info = self.path_cache.get(&context.path.get_key())?;
        context.class_name.clone_from(&info.0);
        context.method_name.clone_from(&info.1);
        let mut class_name = context.class_name.clone();
        if let Some(version) = &context.version {
            class_name.push(':');
            class_name.push_str(version);
        }
        self.cache.get(&class_name).copied()
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
