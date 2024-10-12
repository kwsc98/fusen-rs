use super::{FusenFilter, ProceedingJoinPoint};
use fusen_common::{
    error::FusenError,
    server::RpcServer,
    trie::{QueryResult, Trie},
    FusenContext, FusenFuture, Path,
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
            for method in info.get_methods() {
                let path = method.get_path().clone();
                let name = method.get_name().clone();
                let method = method.get_method().clone();
                if path.contains('{') {
                    rest_trie.insert(path.clone());
                }
                hash_cache.insert(
                    Path::new(&method, path).get_key(),
                    (info.get_id().to_string(), name.clone()),
                );
                hash_cache.insert(
                    Path::new(&method, format!("/{}/{}", info.get_id(), name.clone())).get_key(),
                    (info.get_id().to_string(), name),
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
        let context_info = context.get_context_info();
        let mut class_name = context_info.get_class_name().clone();
        if let Some(version) = context_info.get_version() {
            class_name.push(':');
            class_name.push_str(version);
        }
        self.cache.get(&class_name).copied()
    }
}

impl FusenFilter for RpcServerFilter {
    fn call(
        &self,
        mut join_point: ProceedingJoinPoint,
    ) -> FusenFuture<Result<FusenContext, crate::Error>> {
        let server = self.get_server(join_point.get_mut_context());
        match server {
            Some(server) => {
                Box::pin(async move { Ok(server.invoke(join_point.into_data()).await) })
            }
            None => Box::pin(async move {
                join_point
                    .get_mut_context()
                    .get_mut_response()
                    .set_response(Err(FusenError::NotFind));
                Ok(join_point.into_data())
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
    pub fields: Option<Vec<(String, String)>>,
}

impl PathCache {
    pub fn seach(&self, mut_path: &mut Path) -> Option<PathCacheResult> {
        if let Some(data) = self.path_cache.get(&mut_path.get_key()) {
            Some(PathCacheResult {
                class: data.0.clone(),
                method: data.1.clone(),
                fields: None,
            })
        } else if let Some(rest_data) = self.rest_trie.search(&mut_path.get_path()) {
            let QueryResult { path, query_fields } = rest_data;
            mut_path.update_path(path);
            self.path_cache
                .get(&mut_path.get_key())
                .map(|data| PathCacheResult {
                    class: data.0.clone(),
                    method: data.1.clone(),
                    fields: query_fields,
                })
        } else {
            None
        }
    }
}
