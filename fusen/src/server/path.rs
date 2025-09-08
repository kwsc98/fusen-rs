use crate::protocol::fusen::{request::Path, service::MethodInfo};
use async_recursion::async_recursion;
use std::{collections::HashMap, sync::Arc};
use tokio::sync::RwLock;

#[derive(Debug, Default)]
pub struct PathCache {
    path_cache: HashMap<String, Arc<MethodInfo>>,
    rest_trie: Trie,
}

impl PathCache {
    pub async fn seach(&self, path: &Path) -> Option<QueryResult> {
        let key = format!("{}:{}", path.method, path.path);
        if let Some(method_info) = self.path_cache.get(key.as_str()) {
            Some(QueryResult {
                method_info: method_info.clone(),
                rest_fields: None,
            })
        } else {
            self.rest_trie.search(key.as_str()).await
        }
    }
    pub async fn build(method_infos: Vec<Arc<MethodInfo>>) -> Self {
        let mut rest_trie = Trie::default();
        let mut path_cache = HashMap::new();
        for method_info in method_infos {
            rest_trie.insert(method_info.clone()).await;
            let _ = path_cache.insert(
                format!("{}:{}", method_info.method, method_info.path),
                method_info,
            );
        }
        Self {
            path_cache,
            rest_trie,
        }
    }
}

#[derive(Debug, Default)]
pub struct Trie {
    root: Arc<RwLock<TreeNode>>,
}

unsafe impl Sync for Trie {}
unsafe impl Send for Trie {}

#[derive(Debug, Default)]
struct TreeNode {
    nodes: HashMap<String, Arc<RwLock<TreeNode>>>,
    value: Option<Arc<MethodInfo>>,
}

#[derive(Debug)]
pub struct QueryResult {
    pub method_info: Arc<MethodInfo>,
    pub rest_fields: Option<Vec<(String, String)>>,
}

impl Trie {
    pub async fn insert(&mut self, handler_info: Arc<MethodInfo>) {
        let path = format!("{}:{}", handler_info.method, handler_info.path);
        let paths: Vec<&str> = path.split('/').collect();
        let mut temp = self.root.clone();
        for item in paths {
            let res_node = temp.read().await.nodes.get(item).cloned();
            match res_node {
                Some(node) => {
                    temp = node;
                }
                None => {
                    let new_node = Arc::new(RwLock::new(Default::default()));
                    temp.clone()
                        .write()
                        .await
                        .nodes
                        .insert(item.to_owned(), new_node.clone());
                    temp = new_node;
                }
            }
        }
        let _ = temp.write().await.value.insert(handler_info);
    }

    pub async fn search(&self, path: &str) -> Option<QueryResult> {
        Self::search_by_nodes(path, self.root.clone()).await
    }

    #[async_recursion]
    async fn search_by_nodes(path: &str, mut temp: Arc<RwLock<TreeNode>>) -> Option<QueryResult> {
        let mut rest_fields: Vec<(String, String)> = vec![];
        let paths: Vec<&str> = path.split('/').collect();
        for (idx, item) in paths.iter().enumerate() {
            let res_node = temp.read().await.nodes.get(*item).cloned();
            match res_node {
                Some(node) => {
                    temp = node;
                }
                None => {
                    let temp_path = paths[idx + 1..].join("/").to_string();
                    for entry in temp.read().await.nodes.iter() {
                        if entry.0.starts_with('{') {
                            if temp_path.is_empty() && entry.1.read().await.value.is_some() {
                                rest_fields.push((
                                    entry.0[1..entry.0.len() - 1].to_string(),
                                    item.to_string(),
                                ));
                                return Some(QueryResult {
                                    method_info: entry
                                        .1
                                        .read()
                                        .await
                                        .value
                                        .as_ref()
                                        .unwrap()
                                        .clone(),
                                    rest_fields: if rest_fields.is_empty() {
                                        None
                                    } else {
                                        Some(rest_fields)
                                    },
                                });
                            }
                            if let Some(query_result) =
                                Self::search_by_nodes(&temp_path, entry.1.clone()).await
                            {
                                rest_fields.push((
                                    entry.0[1..entry.0.len() - 1].to_string(),
                                    item.to_string(),
                                ));
                                if let Some(mut temp_query_fields) = query_result.rest_fields {
                                    rest_fields.append(&mut temp_query_fields);
                                }
                                return Some(QueryResult {
                                    method_info: query_result.method_info,
                                    rest_fields: if rest_fields.is_empty() {
                                        None
                                    } else {
                                        Some(rest_fields)
                                    },
                                });
                            }
                        }
                    }
                    return None;
                }
            }
        }
        temp.read()
            .await
            .value
            .as_ref()
            .map(|method_info| QueryResult {
                method_info: method_info.clone(),
                rest_fields: if rest_fields.is_empty() {
                    None
                } else {
                    Some(rest_fields)
                },
            })
    }
}
