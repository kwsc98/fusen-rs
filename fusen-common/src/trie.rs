use async_recursion::async_recursion;
use fusen_procedural_macro::Data;
use std::{collections::HashMap, sync::Arc};
use tokio::sync::RwLock;

#[derive(Debug, Default)]
pub struct Trie {
    root: Arc<RwLock<TreeNode>>,
}

unsafe impl Sync for Trie {}
unsafe impl Send for Trie {}

#[derive(Debug, Default)]
struct TreeNode {
    nodes: HashMap<String, Arc<RwLock<TreeNode>>>,
    value: Option<String>,
}

#[derive(Debug, Data)]
pub struct QueryResult {
    pub path: String,
    pub query_fields: Option<Vec<(String, String)>>,
}

impl Trie {
    pub async fn insert(&mut self, path: String) {
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
        let _ = temp.write().await.value.insert(path);
    }

    pub async fn search(&self, path: &str) -> Option<QueryResult> {
        Self::search_by_nodes(path, self.root.clone()).await
    }

    #[async_recursion]
    async fn search_by_nodes(path: &str, mut temp: Arc<RwLock<TreeNode>>) -> Option<QueryResult> {
        let mut query_fields: Vec<(String, String)> = vec![];
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
                                query_fields.push((
                                    entry.0[1..entry.0.len() - 1].to_string(),
                                    item.to_string(),
                                ));
                                return Some(QueryResult {
                                    path: entry.1.read().await.value.as_ref().unwrap().clone(),
                                    query_fields: if query_fields.is_empty() {
                                        None
                                    } else {
                                        Some(query_fields)
                                    },
                                });
                            }
                            if let Some(query_result) =
                                Self::search_by_nodes(&temp_path, entry.1.clone()).await
                            {
                                query_fields.push((
                                    entry.0[1..entry.0.len() - 1].to_string(),
                                    item.to_string(),
                                ));
                                if let Some(mut temp_query_fields) = query_result.query_fields {
                                    query_fields.append(&mut temp_query_fields);
                                }
                                return Some(QueryResult {
                                    path: query_result.path,
                                    query_fields: if query_fields.is_empty() {
                                        None
                                    } else {
                                        Some(query_fields)
                                    },
                                });
                            }
                        }
                    }
                    return None;
                }
            }
        }
        temp.read().await.value.as_ref().map(|path| QueryResult {
            path: path.clone(),
            query_fields: if query_fields.is_empty() {
                None
            } else {
                Some(query_fields)
            },
        })
    }
}

#[tokio::test]
async fn test() {
    let mut pre_trie = Trie::default();
    pre_trie.insert("/tasks/{tasks_id}/point".to_owned()).await;
    pre_trie
        .insert("/tasks/{tasks_id}/point/{user_id}".to_owned())
        .await;
    pre_trie
        .insert("/tasks/{tasks_id}/point/{user_id}/{merchant_id}".to_owned())
        .await;
    println!("{:?}", pre_trie.search("/tasks/iu321/point").await);
    println!("{:?}", pre_trie.search("/tasks/iu322/point/user9090").await);
    println!(
        "{:?}",
        pre_trie.search("/tasks/iu322/point/user9090/dsdsdsd").await
    );
}
