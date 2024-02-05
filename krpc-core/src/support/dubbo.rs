use crate::register::{Info, Resource};

pub fn decode_url(url: &str) -> Result<Resource, String> {
    let mut url = &krpc_common::url_util::decode_url(url).unwrap()[..];
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
    match resource {
        Resource::Client(info) => {
            let mut url = String::new();
            url.push_str("consumer://");
            url.push_str(&(get_path(info) + &"/"));
            url.push_str(&(info.server_name.clone() + &"?"));
            url.push_str("application=dubbo-springboot-demo-consumer&background=false&category=consumers&check=false&dubbo=2.0.2&executor-management-mode=isolation&file-cache=true&interface=org.apache.dubbo.springboot.demo.DemoService&methods=sayHello&pid=20591&release=3.3.0-beta.1&side=consumer&sticky=false&unloadClusterRelated=false");
            return "/".to_string() + &krpc_common::url_util::encode_url(&url).unwrap();
        }
        Resource::Server(info) => {
            let mut url = String::new();
            url.push_str("tri://");
            url.push_str(&(get_path(info) + &"/"));
            url.push_str(&(info.server_name.clone() + &"?"));
            url.push_str("application=dubbo-springboot-demo-provider&deprecated=false&dubbo=2.0.2&dynamic=true&generic=false&interface=org.apache.dubbo.springboot.demo.DemoService&methods=sayHello&prefer.serialization=fastjson&release=3.2.11&serialization=fastjson&service-name-mapping=true&side=provider&timestamp=1707098182442");
            return "/".to_string() + &krpc_common::url_util::encode_url(&url).unwrap();
        }
    }
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
    let info = Info {
        server_name,
        version: "1.0.0".to_string(),
        ip: path.0,
        port: path.1,
    };
    return info;
}
