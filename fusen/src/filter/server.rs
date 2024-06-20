use super::FusenFilter;
use fusen_common::{
    error::FusenError, server::RpcServer, trie::Trie, FusenContext, FusenFuture, MethodResource,
    Path,
};
use std::{collections::HashMap, sync::Arc};

#[derive(Clone, Default)]
pub struct RpcServerFilter {
    cache: HashMap<String, &'static dyn RpcServer>,
    path_cache: Arc<PathCache>,
}

impl RpcServerFilter {
    pub fn new(cache: HashMap<String, &'static dyn RpcServer>) -> Self {
        let mut hash_cache = HashMap::new();
        let mut rest_trie = Trie::default();
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
                if path.contains('{') {
                    rest_trie.insert(path.clone());
                }
                hash_cache.insert(
                    Path::POST(path_rpc).get_key(),
                    (info.id.to_string(), name.clone()),
                );
                hash_cache.insert(
                    Path::new(&method, path).get_key(),
                    (info.id.to_string(), name),
                );
            }
        }
        RpcServerFilter {
            cache,
            path_cache: Arc::new(PathCache {
                path_cache: hash_cache,
                rest_trie,
            }),
        }
    }
    pub fn get_path_cache(&self) -> Arc<PathCache> {
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

#[derive(Debug, Default)]
pub struct PathCache {
    path_cache: HashMap<String, (String, String)>,
    rest_trie: Trie,
}

pub struct PathCacheResult {
    pub class: String,
    pub method: String,
    pub fields: Option<(Vec<String>, Vec<String>)>,
}

impl PathCache {
    pub fn seach(&self, path: &mut Path) -> Option<PathCacheResult> {
        if let Some(data) = self.path_cache.get(&path.get_key()) {
            Some(PathCacheResult {
                class: data.0.clone(),
                method: data.1.clone(),
                fields: None,
            })
        } else if let Some(rest_data) = self.rest_trie.search(&path.get_path()) {
            path.update_path(rest_data.path);
            if let Some(data) = self.path_cache.get(&path.get_key()) {
                Some(PathCacheResult {
                    class: data.0.clone(),
                    method: data.1.clone(),
                    fields: rest_data.fields,
                })
            } else {
                None
            }
        } else {
            None
        }
    }
}
