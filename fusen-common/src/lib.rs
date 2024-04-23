use std::collections::{hash_map::Iter, HashMap};
use codec::CodecType;
use error::FusenError;
use http::{HeaderMap, HeaderValue};
use register::Type;
use serde::{Deserialize, Serialize};
pub type Error = Box<dyn std::error::Error + Send + Sync>;
pub type Result<T> = std::result::Result<T, Error>;
pub type Response<T> = std::result::Result<T, String>;
pub type FusenFuture<T> = std::pin::Pin<Box<dyn std::future::Future<Output = T> + Send>>;
pub type FusenResult<T> = std::result::Result<T, FusenError>;
pub mod codec;
pub mod date_util;
pub mod error;
pub mod logs;
pub mod r#macro;
pub mod net;
pub mod server;
pub mod url;
pub mod register;

#[derive(Debug)]
pub struct MetaData {
    pub inner: HashMap<String, String>,
}

impl MetaData {
    pub fn get_codec(&self) -> CodecType {
        let content_type = self.get_value("content-type");
        if let Some(str) = content_type {
            if str.to_lowercase().contains("grpc") {
                return CodecType::GRPC;
            }
        }
        return CodecType::JSON;
    }

    pub fn get_value(&self, key: &str) -> Option<&String> {
        self.inner.get(key)
    }

    pub fn get_iter(&self) -> Iter<String, String> {
        self.inner.iter()
    }
    pub fn clone_map(&self) -> HashMap<String, String> {
        self.inner.clone()
    }

    pub fn insert(&mut self, key: String, value: String) -> Option<String> {
        self.inner.insert(key, value)
    }
    pub fn remove(&mut self, key: &str) -> Option<String> {
        self.inner.remove(key)
    }
}

impl From<&HeaderMap<HeaderValue>> for MetaData {
    fn from(value: &HeaderMap<HeaderValue>) -> MetaData {
        value.iter().fold(MetaData::new(), |mut meta, e| {
            meta.inner
                .insert(e.0.to_string(), e.1.to_str().unwrap().to_string());
            meta
        })
    }
}

impl MetaData {
    pub fn new() -> Self {
        MetaData::default()
    }
}

impl Default for MetaData {
    fn default() -> Self {
        let mut inner = HashMap::new();
        inner.insert("prefer.serialization".to_owned(), "fastjson".to_owned());
        inner.insert("protocol".to_owned(), "tri".to_owned());
        Self { inner }
    }
}

#[derive(Debug)]
pub struct FusenContext {
    pub unique_identifier: String,
    pub path: Path,
    pub server_tyep : Type, 
    pub meta_data: MetaData,
    pub class_name: String,
    pub method_name: String,
    pub version: Option<String>,
    pub group: Option<String>,
    pub req: Vec<String>,
    pub fields: Vec<String>,
    pub res: core::result::Result<String, FusenError>,
}

impl FusenContext {
    pub fn new(
        unique_identifier: String,
        path: Path,
        meta_data: MetaData,
        version: Option<String>,
        group: Option<String>,
        class_name: String,
        method_name: String,
        req: Vec<String>,
        fields: Vec<String>,
        return_ty : Option<&str>
    ) -> FusenContext {
        return FusenContext {
            unique_identifier,
            path,
            server_tyep : Type::default(),
            meta_data,
            version,
            group,
            class_name,
            method_name,
            req,
            fields,
            res: Err(FusenError::Null),
        };
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MethodResource {
    pub id: String,
    pub name: String,
    pub path: String,
    pub method: String,
}

#[derive(Debug)]
pub enum Path {
    GET(String),
    POST(String),
}

impl Path {
    pub fn get_key(&self) -> String {
        let mut key = String::new();
        match self {
            Path::GET(path) => {
                key.push_str("get:");
                key.push_str(&path);
            }
            Path::POST(path) => {
                key.push_str("post:");
                key.push_str(&path);
            }
        };
        key
    }

    pub fn new(method: &str, path: String) -> Self {
        if method.to_lowercase().contains("get") {
            Self::GET(path)
        } else {
            Self::POST(path)
        }
    }
}

impl MethodResource {
    pub fn get_id(&self) -> String {
        return self.id.to_string();
    }
    pub fn get_name(&self) -> String {
        return self.name.to_string();
    }
    pub fn get_path(&self) -> String {
        return self.path.to_string();
    }
    pub fn get_method(&self) -> String {
        return self.method.to_string();
    }
    pub fn new(id: String, name: String, path: String, method: String) -> Self {
        Self {
            id,
            name,
            path,
            method,
        }
    }
    pub fn form_json_str(str: &str) -> Self {
        serde_json::from_str(str).unwrap()
    }
    pub fn to_json_str(&self) -> String {
        serde_json::to_string(self).unwrap()
    }
}
