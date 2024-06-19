use std::{cell::RefCell, collections::HashMap, rc::Rc};

#[derive(Default)]
pub struct Trie {
    root: Rc<RefCell<TreeNode>>,
}

#[derive(Default)]
struct TreeNode {
    nodes: HashMap<String, Rc<RefCell<TreeNode>>>,
    value: Option<String>,
}

impl Trie {
    pub fn insert(&mut self, path: String) {
        let paths: Vec<&str> = path.split('/').collect();
        let mut temp = self.root.clone();
        for item in paths {
            if let Some(node) = temp.clone().borrow().nodes.get(item).cloned() {
                temp = node.clone();
            } else {
                let new_node = Rc::new(RefCell::new(Default::default()));
                temp.as_ref()
                    .borrow_mut()
                    .nodes
                    .insert(item.to_owned(), new_node.clone());
                temp = new_node;
            };
        }
        let _ = temp.as_ref().borrow_mut().value.insert(path);
    }

    pub fn search(&self, path: &str) -> Option<String> {
        todo!()
    }
}
