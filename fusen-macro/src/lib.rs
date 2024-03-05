use proc_macro::TokenStream;
use syn::{Attribute, Meta};
use std::collections::HashMap;

mod trait_macro;
mod server_macro;


#[proc_macro_attribute]
pub fn fusen_trait(attr: TokenStream, item: TokenStream) -> TokenStream {
    trait_macro::fusen_trait(attr, item)
}

#[proc_macro_attribute]
pub fn fusen_server(attr: TokenStream, item: TokenStream) -> TokenStream {
   server_macro::fusen_server(attr, item)
}

#[proc_macro_attribute]
pub fn resource(_attr: TokenStream, item: TokenStream) -> TokenStream {
    item
}

fn parse_attr(attr: TokenStream) -> HashMap<String,String> {
    let mut map = HashMap::new();
    let attr = attr.clone().to_string();
    let args: Vec<&str> = attr.split(",").collect();
    for arg in args {
        let arg = arg.replace(" ", "");
        let item: Vec<&str> = arg.split("=").collect();
        map.insert(
            item[0].to_string().clone(),
            item[1].replace("\"", "").to_string().clone(),
        );
    }
    return map;
}

fn get_resource_by_attrs(attrs: &Vec<Attribute>) -> (Option<String>, Option<String>) {
    let mut parent_path = None;
    let mut parent_method = None;
    for attr in attrs {
        if let Meta::List(list) = &attr.meta {
            if let Some(segment) = list.path.segments.first() {
                if &segment.ident.to_string() == &"resource" {
                    let temp_map = parse_attr(list.tokens.clone().into());
                    if let Some(path) = temp_map.get("path") {
                        let _ = parent_path.insert(path.to_string());
                    }
                    if let Some(method) = temp_map.get("method") {
                        let _ = parent_method.insert(method.to_string());
                    }
                }
            }
        }
    }
    (parent_path, parent_method)
}