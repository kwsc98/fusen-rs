use std::{cell::RefCell, collections::HashMap, rc::Rc};

#[derive(Debug, Default)]
pub struct Trie {
    root: Rc<RefCell<TreeNode>>,
}

unsafe impl Sync for Trie {}
unsafe impl Send for Trie {}

#[derive(Debug, Default)]
struct TreeNode {
    nodes: HashMap<String, Rc<RefCell<TreeNode>>>,
    value: Option<String>,
}

#[derive(Debug)]
pub struct QueryResult {
    pub path: String,
    pub fields: Option<(Vec<String>, Vec<String>)>,
}

impl Trie {
    pub fn insert(&mut self, path: String) {
        let paths: Vec<&str> = path.split('/').collect();
        let mut temp = self.root.clone();
        for item in paths {
            let res_node = temp.as_ref().borrow().nodes.get(item).cloned();
            match res_node {
                Some(node) => {
                    temp = node;
                }
                None => {
                    let new_node = Rc::new(RefCell::new(Default::default()));
                    temp.clone()
                        .borrow_mut()
                        .nodes
                        .insert(item.to_owned(), new_node.clone());
                    temp = new_node;
                }
            }
        }
        let _ = temp.as_ref().borrow_mut().value.insert(path);
    }

    pub fn search(&self, path: &str) -> Option<QueryResult> {
        Self::search_by_nodes(path, self.root.clone())
    }

    fn search_by_nodes(path: &str, mut temp: Rc<RefCell<TreeNode>>) -> Option<QueryResult> {
        let mut fields: Vec<String> = vec![];
        let mut fields_ty: Vec<String> = vec![];
        let paths: Vec<&str> = path.split('/').collect();
        for (idx, item) in paths.iter().enumerate() {
            let res_node = temp.as_ref().borrow().nodes.get(*item).cloned();
            match res_node {
                Some(node) => {
                    temp = node;
                }
                None => {
                    let temp_path = paths[idx + 1..].join("/").to_string();
                    for entry in temp.as_ref().borrow().nodes.iter() {
                        if entry.0.starts_with('{') {
                            if temp_path.is_empty() && entry.1.as_ref().borrow().value.is_some() {
                                fields.push(item.to_string());
                                fields_ty.push(entry.0[1..entry.0.len() - 1].to_string());
                                return Some(QueryResult {
                                    path: entry.1.as_ref().borrow().value.as_ref().unwrap().clone(),
                                    fields: if fields.is_empty() {
                                        None
                                    } else {
                                        Some((fields, fields_ty))
                                    },
                                });
                            }
                            if let Some(query_result) =
                                Self::search_by_nodes(&temp_path, entry.1.clone())
                            {
                                fields.push(item.to_string());
                                fields_ty.push(entry.0[1..entry.0.len() - 1].to_string());
                                if let Some((mut temp_fields, mut temp_fields_ty)) =
                                    query_result.fields
                                {
                                    fields.append(&mut temp_fields);
                                    fields_ty.append(&mut temp_fields_ty);
                                }
                                return Some(QueryResult {
                                    path: query_result.path,
                                    fields: if fields.is_empty() {
                                        None
                                    } else {
                                        Some((fields, fields_ty))
                                    },
                                });
                            }
                        }
                    }
                    return None;
                }
            }
        }
        temp.as_ref()
            .borrow()
            .value
            .as_ref()
            .map(|path| QueryResult {
                path: path.clone(),
                fields: if fields.is_empty() {
                    None
                } else {
                    Some((fields, fields_ty))
                },
            })
    }
}

#[test]
fn test() {
    let mut pre_trie = Trie::default();
    pre_trie.insert("/tasks/{tasks_id}/point".to_owned());
    pre_trie.insert("/tasks/{tasks_id}/point/{user_id}".to_owned());
    pre_trie.insert("/tasks/{tasks_id}/point/{user_id}/{merchant_id}".to_owned());
    println!("{:?}", pre_trie.search("/tasks/iu321/point"));
    println!("{:?}", pre_trie.search("/tasks/iu322/point/user9090"));
    println!(
        "{:?}",
        pre_trie.search("/tasks/iu322/point/user9090/dsdsdsd")
    );
}
