use bytes::{Bytes, BytesMut};
use codec::CodecType;
use error::FusenError;
use fusen_procedural_macro::Data;
use http::{HeaderMap, HeaderValue};
use register::Type;
use serde::{Deserialize, Serialize};
use std::collections::{hash_map::Iter, HashMap};
pub type Error = Box<dyn std::error::Error + Send + Sync>;
pub type Result<T> = std::result::Result<T, Error>;
pub type Response<T> = std::result::Result<T, String>;
pub type FusenFuture<T> = std::pin::Pin<Box<dyn std::future::Future<Output = T> + Send>>;
pub type FusenResult<T> = std::result::Result<T, FusenError>;
pub mod codec;
pub mod config;
pub mod date_util;
pub mod error;
pub mod logs;
pub mod r#macro;
pub mod net;
pub mod register;
pub mod server;
pub mod trie;
pub mod url;

#[derive(Debug, Data)]
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
        CodecType::JSON
    }
    pub fn into_inner(self) -> HashMap<String, String> {
        self.inner
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
        inner.insert(
            "preserved.register.source".to_owned(),
            "SPRING_CLOUD".to_owned(),
        );
        inner.insert("protocol".to_owned(), "tri".to_owned());
        Self { inner }
    }
}

#[derive(Debug, Default, Data)]
pub struct ContextInfo {
    path: Path,
    class_name: String,
    method_name: String,
    version: Option<String>,
    group: Option<String>,
}

impl ContextInfo {
    pub fn new(
        path: Path,
        class_name: String,
        method_name: String,
        version: Option<String>,
        group: Option<String>,
    ) -> Self {
        ContextInfo {
            path,
            class_name,
            method_name,
            version,
            group,
        }
    }
    pub fn get_handler_key(&self) -> String {
        let mut key = self.class_name.clone();
        if let Some(version) = &self.version {
            key.push(':');
            key.push_str(version);
        }
        key
    }
}

#[derive(Debug, Data)]
pub struct FusenRequest {
    query_fields: Option<Vec<(String, String)>>,
    body: Bytes,
}

impl FusenRequest {
    pub fn new_for_client(method: &str, fields_ty: Vec<String>, bodys: Vec<String>) -> Self {
        let mut query_fields = None;
        let mut bytes = BytesMut::new();
        if method.to_lowercase().as_str() != "post" {
            let mut vec = vec![];
            for (idx, body) in bodys.into_iter().enumerate() {
                vec.push((fields_ty[idx].to_owned(), body));
            }
            let _ = query_fields.insert(vec);
        } else {
            bytes.extend_from_slice(serde_json::to_string(&bodys).unwrap().as_bytes());
        }
        FusenRequest {
            query_fields,
            body: bytes.into(),
        }
    }
    pub fn new(query_fields: Option<Vec<(String, String)>>, body: Bytes) -> Self {
        FusenRequest { query_fields, body }
    }
    pub fn get_fields(
        &mut self,
        temp_fields_name: Vec<&str>,
        temp_fields_ty: Vec<&str>,
    ) -> Result<Vec<String>> {
        let mut new_fields = vec![];
        if let Some(fields_name) = &self.query_fields {
            let mut hash_map = HashMap::with_capacity(8);
            for item in fields_name.iter() {
                let _ = hash_map.insert(item.0.clone(), item.1.clone());
            }
            for item in temp_fields_name.iter().enumerate() {
                let fields = hash_map.get(*item.1).ok_or("fields handler error")?;
                let mut temp = String::new();
                if "String" == temp_fields_ty[item.0] {
                    temp.push('\"');
                    temp.push_str(fields);
                    temp.push('\"');
                } else {
                    temp.push_str(fields);
                }
                new_fields.push(temp);
            }
        } else if self.body.starts_with(b"[") {
            new_fields = serde_json::from_slice(&self.body)?;
        } else {
            new_fields.push(String::from_utf8(self.body.to_vec())?);
        }
        Ok(new_fields)
    }
}

#[derive(Debug, Data)]
pub struct FusenResponse {
    response: std::result::Result<Bytes, FusenError>,
    response_ty: Option<&'static str>,
}

impl Default for FusenResponse {
    fn default() -> Self {
        Self {
            response: Err(FusenError::Null),
            response_ty: Default::default(),
        }
    }
}

impl FusenResponse {
    pub fn insert_return_ty(&mut self, ty: &'static str) {
        let _ = self.response_ty.insert(ty);
    }
    pub fn into_response(self) -> std::result::Result<Bytes, FusenError> {
        self.response
    }
}

#[derive(Debug, Data)]
pub struct FusenContext {
    unique_identifier: String,
    server_type: Type,
    meta_data: MetaData,
    context_info: ContextInfo,
    request: FusenRequest,
    response: FusenResponse,
}

impl FusenContext {
    pub fn new(
        unique_identifier: String,
        context_info: ContextInfo,
        request: FusenRequest,
        meta_data: MetaData,
    ) -> FusenContext {
        FusenContext {
            unique_identifier,
            context_info,
            server_type: Type::Fusen,
            meta_data,
            request,
            response: Default::default(),
        }
    }
    pub fn insert_server_type(&mut self, server_tyep: Type) {
        self.server_type = server_tyep
    }
    pub fn into_response(self) -> FusenResponse {
        self.response
    }
    pub fn get_return_ty(&self) -> Option<&'static str> {
        self.response.response_ty
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MethodResource {
    name: String,
    path: String,
    method: String,
}

#[derive(Debug, Clone)]
pub enum Path {
    GET(String),
    PUT(String),
    DELETE(String),
    POST(String),
}

impl Default for Path {
    fn default() -> Self {
        Path::GET(Default::default())
    }
}

impl Path {
    pub fn get_key(&self) -> String {
        let mut key = String::new();
        match self {
            Path::GET(path) => {
                key.push_str("get:");
                key.push_str(path);
            }
            Path::POST(path) => {
                key.push_str("post:");
                key.push_str(path);
            }
            Path::PUT(path) => {
                key.push_str("put:");
                key.push_str(path);
            }
            Path::DELETE(path) => {
                key.push_str("delete:");
                key.push_str(path);
            }
        };
        key
    }

    pub fn update_path(&mut self, new_path: String) {
        match self {
            Path::GET(path) => *path = new_path,
            Path::POST(path) => *path = new_path,
            Path::PUT(path) => *path = new_path,
            Path::DELETE(path) => *path = new_path,
        }
    }

    pub fn get_path(&self) -> String {
        match self {
            Path::GET(path) => path,
            Path::POST(path) => path,
            Path::PUT(path) => path,
            Path::DELETE(path) => path,
        }
        .clone()
    }

    pub fn new(method: &str, path: String) -> Self {
        match method.to_lowercase().as_str() {
            "get" => Self::GET(path),
            "put" => Self::PUT(path),
            "delete" => Self::DELETE(path),
            _ => Self::POST(path),
        }
    }
}

impl MethodResource {
    pub fn get_name(&self) -> String {
        self.name.to_string()
    }
    pub fn get_path(&self) -> String {
        self.path.to_string()
    }
    pub fn get_method(&self) -> String {
        self.method.to_string()
    }
    pub fn new(name: String, path: String, method: String) -> Self {
        Self { name, path, method }
    }
    pub fn new_macro(method_str: &str) -> Self {
        let method: Vec<String> = serde_json::from_str(method_str).unwrap();
        Self {
            name: method[0].to_string(),
            path: method[1].to_string(),
            method: method[2].to_string(),
        }
    }
    pub fn form_json_str(str: &str) -> Self {
        serde_json::from_str(str).unwrap()
    }
    pub fn to_json_str(&self) -> String {
        serde_json::to_string(self).unwrap()
    }
}
