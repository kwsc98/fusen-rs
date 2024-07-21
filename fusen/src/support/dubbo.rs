use fusen_common::MethodResource;

use crate::register::{Category, Resource};
use std::{collections::HashMap, vec};

pub fn decode_url(url: &str) -> Result<Resource, crate::Error> {
    let url = &fusen_common::url::decode_url(url)?[..];
    get_info(url)
}

pub fn encode_url(resource: &Resource) -> String {
    let mut url = String::new();
    match resource.category {
        Category::Client => url.push_str("consumer://"),
        Category::Service => url.push_str("tri://"),
        Category::Server => (),
    }
    url.push_str(&(get_path(resource) + "/"));
    url.push_str(&(resource.server_name.clone() + "?"));
    url.push_str(&("interface=".to_owned() + &resource.server_name));
    url.push_str(&get_field_url(
        "&methods",
        &resource.methods.iter().fold(vec![], |mut vec, e| {
            vec.push(e.get_id());
            vec
        }),
    ));
    if let Some(version) = &resource.version {
        let value = vec![version.clone()];
        url.push_str(&get_field_url("&version", &value));
    }
    match resource.category {
        Category::Client => url.push_str("&dubbo=2.0.2&release=3.3.0-beta.1&side=consumer"),
        Category::Service => url.push_str(
            "&dubbo=2.0.2&prefer.serialization=fastjson&release=3.3.0-beta.1&side=provider",
        ),
        _ => (),
    }
    "/".to_string() + &fusen_common::url::encode_url(&url)
}

fn get_ip(path: &str) -> (String, Option<String>) {
    let path: Vec<&str> = path.split(':').collect();
    let mut port = None;
    if path.len() > 1 {
        let _ = port.insert(path[1].to_string());
    }
    (path[0].to_string(), port)
}

fn get_path(info: &Resource) -> String {
    let mut ip = info.ip.clone();
    if let Some(port) = info.port.clone() {
        ip.push(':');
        ip.push_str(&port);
    }
    ip.to_string()
}

fn get_info(mut url: &str) -> crate::Result<Resource> {
    let mut category = Category::Server;
    if url.starts_with("tri://") {
        url = &url[6..];
    } else if url.starts_with("consumer://") {
        url = &url[11..];
        category = Category::Client;
    } else {
        return Err(format!("err url : {}", url).into());
    }
    let info: Vec<&str> = url.split('/').collect();
    let path = get_ip(info[0]);
    let info: Vec<&str> = info[1].split('?').collect();
    let server_name = info[0].to_string();
    let vision = get_field_values(info[1], "version");
    let mut revision = None;
    if !vision.is_empty() {
        let _ = revision.insert(vision[0].clone());
    }
    let group = get_field_values(info[1], "group");
    let mut regroup = None;
    if !group.is_empty() {
        let _ = regroup.insert(vision[0].clone());
    }
    let info = Resource {
        server_name,
        category,
        group: regroup,
        version: revision,
        methods: get_field_values(info[1], "methods")
            .iter()
            .fold(vec![], |mut vec, e| {
                vec.push(MethodResource::new(
                    e.to_string(),
                    e.to_string(),
                    "/".to_owned() + e,
                    "POST".to_owned(),
                ));
                vec
            }),
        ip: path.0,
        port: path.1,
        params: HashMap::new(),
    };
    Ok(info)
}

fn get_field_values(str: &str, key: &str) -> Vec<String> {
    let fields: Vec<&str> = str.split('&').collect();
    let mut res = vec![];
    for field in fields {
        let field: Vec<&str> = field.split('=').collect();
        if field[0] == key {
            let velues: Vec<&str> = field[1].split(',').collect();
            res = velues.iter().fold(res, |mut res, &e| {
                res.push(e.to_string());
                res
            });
            break;
        }
    }
    res
}

fn get_field_url(key: &str, values: &Vec<String>) -> String {
    if values.is_empty() {
        return String::new();
    }
    let mut res = String::new();
    for value in values {
        res.push_str(&(value.to_owned() + ","));
    }
    key.to_string() + "=" + &res[..res.len() - 1]
}
