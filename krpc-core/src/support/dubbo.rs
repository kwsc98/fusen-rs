use std::vec;
use crate::register::{Info, Resource};

pub fn decode_url(url: &str) -> Result<Resource, String> {
    let mut url = &krpc_common::url_util::decode_url(url)?[..];
    if url.starts_with("tri://") {
        url = &url[6..];
        return Ok(Resource::Server(get_info(url)));
    } else if url.starts_with("consumer://") {
        url = &url[11..];
        return Ok(Resource::Client(get_info(url)));
    }
    return Err("decode err".to_string());
}

pub fn encode_url(resource: &Resource) -> String {
    let mut url = String::new();
    match resource {
        Resource::Client(info) => {
            url.push_str("consumer://");
            url.push_str(&(get_path(info) + &"/"));
            url.push_str(&(info.server_name.clone() + &"?"));
            url.push_str(&("interface=".to_owned() + &info.server_name));
            url.push_str(&get_field_url("&methods", &info.methods));
            if let Some(version) = &info.version {
                let value = vec![version.clone()];
                url.push_str(&get_field_url("&version", &value));
            }
            url.push_str("&dubbo=2.0.2&release=3.3.0-beta.1&side=consumer");
        }
        Resource::Server(info) => {
            url.push_str("tri://");
            url.push_str(&(get_path(info) + &"/"));
            url.push_str(&(info.server_name.clone() + &"?"));
            url.push_str(&("interface=".to_owned() + &info.server_name));
            url.push_str(&get_field_url("&methods", &info.methods));
            if let Some(version) = &info.version {
                let value = vec![version.clone()];
                url.push_str(&get_field_url("&version", &value));
            }
            url.push_str(
                "&dubbo=2.0.2&prefer.serialization=fastjson&release=3.3.0-beta.1&side=provider",
            );
        }
    }
    return "/".to_string() + &krpc_common::url_util::encode_url(&url);
}

fn get_ip(path: &str) -> (String, Option<String>) {
    let path: Vec<&str> = path.split(":").collect();
    let mut port = None;
    if path.len() > 1 {
        let _ = port.insert(path[1].to_string());
    }
    return (path[0].to_string(), port);
}

fn get_path(info: &Info) -> String {
    let mut ip = info.ip.clone();
    if let Some(port) = info.port.clone() {
        ip.push_str(":");
        ip.push_str(&port);
    }
    return ip.to_string();
}

fn get_info(url: &str) -> Info {
    let info: Vec<&str> = url.split("/").collect();
    let path = get_ip(info[0]);
    let info: Vec<&str> = info[1].split("?").collect();
    let server_name = info[0].to_string();
    let vision = get_field_values(info[1], "version");
    let mut revision = None;
    if !vision.is_empty(){
        let _ = revision.insert(vision[0].clone());
    }
    let info = Info {
        server_name,
        version: revision,
        methods: get_field_values(info[1], "methods"),
        ip: path.0,
        port: path.1,
    };
    return info;
}

fn get_field_values(str: &str, key: &str) -> Vec<String> {
    let fields: Vec<&str> = str.split("&").collect();
    let mut res = vec![];
    for field in fields {
        let field: Vec<&str> = field.split("=").collect();
        if field[0] == key {
            let velues: Vec<&str> = field[1].split(",").collect();
            res = velues.iter().fold(res, |mut res, &e| {
                res.push(e.to_string());
                res
            });
            break;
        }
    }
    return res;
}

fn get_field_url(key: &str, values: &Vec<String>) -> String {
    if values.is_empty() {
        return String::new();
    }
    let mut res = String::new();
    for value in values {
        res.push_str(&(value.to_owned() + ","));
    }
    return key.to_string() + "=" + &res[..res.len() - 1];
}
