use std::collections::HashMap;

use bytes::Buf;
use percent_encoding::{self, percent_decode_str, percent_encode, AsciiSet, CONTROLS};
use serde::{Deserialize, Serialize};
const FRAGMENT: &AsciiSet = &CONTROLS
    .add(b':')
    .add(b'/')
    .add(b'&')
    .add(b'?')
    .add(b'=')
    .add(b',');

pub fn decode_url(url: &str) -> Result<String, String> {
    Ok(percent_decode_str(url)
        .decode_utf8()
        .map_err(|e| e.to_string())?
        .to_string())
}
pub fn encode_url(url: &str) -> String {
    percent_encode(url.as_bytes(), FRAGMENT).to_string()
}

pub fn from_url<'a, T: Deserialize<'a>>(url: &str) -> Result<T, crate::Error> {
    let info: Vec<&str> = url.split("&").collect();
    let mut map = HashMap::new();
    for item in info {
        let item: Vec<&str> = item.split("=").collect();
        map.insert(item[0], item[1]);
    }
    let json_str = serde_json::to_vec(&map)?;
    let mut deserializer = serde_json::Deserializer::from_reader(json_str.reader());
    T::deserialize(&mut deserializer).map_err(|e| e.to_string().into())
}

pub fn to_url<T: Serialize>(t: &T) -> Result<String, crate::Error> {
    let value = serde_json::to_value(t)?;
    let mut str = String::new();
    for item in value.as_object().map_or(Err("err serialize"), |e| Ok(e))? {
        if let Some(value) = item.1.as_str() {
            str.push('&');
            str.push_str(item.0);
            str.push('=');
            str.push_str(value);
        }
    }
    if str.len() > 0 {
        str.remove(0);
    }
    return Ok(str);
}

pub trait UrlConfig {
    fn to_url(&self) -> Result<String, crate::Error>;
    fn boxed(self) -> Box<dyn UrlConfig>;
}
