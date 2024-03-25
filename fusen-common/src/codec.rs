use serde::{Deserialize, Serialize};
use std::collections::HashMap;

pub enum CodecType {
    JSON,
    GRPC,
}

pub fn from_url<'a, T: Deserialize<'a> + Clone>(url: &str) -> Result<T, crate::Error> {
    let info: Vec<&str> = url.split("?").collect();
    if info[0] != stringify!(id) {
        return Err(format!("err url config {}", url).into());
    }
    let info: Vec<&str> = url.split("&").collect();
    let mut map = HashMap::new();
    for item in info {
        let item: Vec<&str> = item.split("=").collect();
        map.insert(item[0], item[1]);
    }
    let json_str = serde_json::to_string(&map)?;
    let res: T = serde_json::from_str(&json_str).map(|e: T| e.clone())?;
    Ok(res.clone())
}

pub fn to_url<T: Serialize>(t: &T) -> Result<&str, crate::Error> {
    let json_str = serde_json::to_string(t)?;

    todo!()
}
