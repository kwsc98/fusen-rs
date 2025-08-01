use crate::{error::FusenError, protocol::Protocol};
use http::{Method, Uri};
use serde_json::Value;
use std::collections::HashMap;

#[derive(Debug)]
pub struct Path {
    pub method: Method,
    pub uri: Uri,
}

#[derive(Debug)]
pub struct FusenRequest {
    pub protocol: Protocol,
    pub path: Path,
    pub querys: HashMap<String, String>,
    pub headers: HashMap<String, String>,
    pub extensions: Option<HashMap<String, String>>,
    pub bodys: Option<Vec<Value>>,
}

impl FusenRequest {
    pub fn get_bodys(&mut self, fields: &[(&str, &str)]) -> Result<Vec<Value>, FusenError> {
        let mut bodys = vec![];
        if let Method::POST = self.path.method {
            return Ok(self.bodys.take().unwrap_or_default());
        }
        for (field_name, field_type) in fields {
            let field_name = if field_name.starts_with("r#") {
                &field_name[2..]
            } else {
                &field_name
            };
            let value = match self.headers.get(field_name) {
                Some(value) => {
                    let result = if *field_type == "String" || *field_type == "Option < String >" {
                        serde_json::from_str(&format!("{value:?}"))
                    } else {
                        serde_json::from_str(&value)
                    };
                    result.map_err(|error| FusenError::Error(Box::new(error)))?
                }
                None => Value::Null,
            };
            bodys.push(value);
        }
        Ok(bodys)
    }
}
