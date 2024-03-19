use std::collections::HashMap;

use codec::CodecType;
use error::FusenError;
use http::{HeaderMap, HeaderValue};
use serde::{Deserialize, Serialize};

pub type Error = Box<dyn std::error::Error + Send + Sync>;
pub type Result<T> = std::result::Result<T, Error>;
pub type Response<T> = std::result::Result<T, String>;
pub type FusenFuture<T> = std::pin::Pin<Box<dyn std::future::Future<Output = T> + Send>>;
pub type FusenResult<T> = std::result::Result<T, FusenError>;
pub mod codec;
pub mod date_util;
pub mod error;
pub mod logs_util;
pub mod r#macro;
pub mod net_util;
pub mod server;
pub mod url_util;

#[derive(Debug)]
pub struct MetaData {
    inner: HashMap<String, String>,
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
        MetaData {
            inner: HashMap::new(),
        }
    }
}

#[derive(Debug)]
pub struct FusenContext {
    pub unique_identifier: String,
    pub path: String,
    pub meta_data: MetaData,
    pub class_name: Option<String>,
    pub method_name: Option<String>,
    pub version: Option<String>,
    pub req: Vec<String>,
    pub res: core::result::Result<String, FusenError>,
}

impl FusenContext {
    pub fn new(
        unique_identifier: String,
        path: String,
        meta_data: MetaData,
        version: Option<String>,
        class_name: Option<String>,
        method_name: Option<String>,
        req: Vec<String>,
    ) -> FusenContext {
        return FusenContext {
            unique_identifier,
            path,
            meta_data,
            version,
            class_name,
            method_name,
            req,
            res: Err(FusenError::Null),
        };
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MethodResource {
    id: String,
    name: String,
    path: String,
    method: String,
}

impl MethodResource {
    pub fn into(self) -> (String, String, String) {
        return (self.id, self.path, self.name);
    }
    pub fn get_id(&self) -> String {
        return self.id.to_string();
    }
    pub fn get_name(&self) -> String {
        return self.name.to_string();
    }
    pub fn get_path(&self) -> String {
        return self.path.to_string();
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
