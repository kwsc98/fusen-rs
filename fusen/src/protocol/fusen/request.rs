use crate::{error::FusenError, protocol::Protocol};
use http::Method;
use serde_json::Value;
use std::{
    collections::{HashMap, LinkedList},
    str::FromStr,
};

#[derive(Debug)]
pub struct Path {
    pub method: Method,
    pub path: String,
}

#[derive(Debug)]
pub struct FusenRequest {
    pub protocol: Protocol,
    pub path: Path,
    pub addr: Option<String>,
    pub querys: HashMap<String, String>,
    pub headers: HashMap<String, String>,
    pub extensions: Option<HashMap<String, String>>,
    pub bodys: Option<LinkedList<Value>>,
}

impl FusenRequest {
    pub fn get_bodys(&mut self, fields: &[(&str, &str)]) -> Result<LinkedList<Value>, FusenError> {
        let mut bodys = LinkedList::new();
        if let Method::POST = self.path.method {
            return Ok(self.bodys.take().unwrap_or_default());
        }
        for (field_name, field_type) in fields {
            let field_name = if let Some(field_name) = field_name.strip_prefix("r#") {
                field_name
            } else {
                field_name
            };
            let value = match self.querys.get(field_name) {
                Some(value) => {
                    let result = if *field_type == "String" || *field_type == "Option < String >" {
                        serde_json::from_str(&format!("{value:?}"))
                    } else {
                        serde_json::from_str(value)
                    };
                    result.map_err(|error| FusenError::Error(Box::new(error)))?
                }
                None => Value::Null,
            };
            bodys.push_back(value);
        }
        Ok(bodys)
    }

    pub fn init_request(
        protocol: Protocol,
        method: &str,
        path: &str,
        field_pats: &[&str],
        mut request_bodys: LinkedList<Value>,
    ) -> Result<Self, FusenError> {
        let method =
            Method::from_str(method).map_err(|error| FusenError::Error(Box::new(error)))?;
        let mut bodys = None;
        let mut querys = HashMap::new();
        if let Method::POST = method {
            let _ = bodys.insert(request_bodys);
        } else {
            for field_pat in field_pats.iter().rev() {
                let field_name = if let Some(field_pat) = field_pat.strip_prefix("r#") {
                    field_pat
                } else {
                    field_pat
                };
                let value = request_bodys.pop_back().unwrap();
                if value.is_null() {
                    continue;
                }
                let mut query_value = value.to_string();
                if value.is_string() {
                    query_value = query_value[1..query_value.len() - 1].to_string();
                }
                querys.insert(field_name.to_owned(), query_value);
            }
        }
        Ok(Self {
            protocol,
            path: Path {
                method,
                path: path.to_owned(),
            },
            addr: None,
            querys,
            headers: Default::default(),
            extensions: Default::default(),
            bodys,
        })
    }
}
