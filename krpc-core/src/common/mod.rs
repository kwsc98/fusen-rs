pub mod date_util;
use serde::{Deserialize, Serialize};
use std::marker::PhantomData;


#[derive(Serialize, Deserialize)]
pub struct KrpcRequest<Req, Res> {
    #[serde(default)]
    pub req: Req,

    pub resource: KrpcResource<Req, Res>,
}

#[derive(Serialize, Deserialize)]
pub struct KrpcResource<Req, Res> {
    #[serde(default)]
    _req: PhantomData<Req>,
    #[serde(default)]
    _res: PhantomData<Res>,
    #[serde(default)]
    version: String,
    #[serde(default)]
    class_name: String,
    #[serde(default)]
    method_name: String,
}

impl<Req, Res> KrpcRequest<Req, Res> {
    pub fn new(resource: &KrpcResource<Req, Res>, req: Req) -> Self {
        return Self {
            req,
            resource: resource.clone(),
        };
    }
}

impl<Req, Res> KrpcResource<Req, Res> {
    pub fn new(version: &str, class_name: &str, method_name: &str) -> Self {
        return Self {
            _req: PhantomData,
            _res: PhantomData,
            version: version.to_string(),
            class_name: class_name.to_string(),
            method_name: method_name.to_string(),
        };
    }
    pub fn clone(&self) -> Self {
        return KrpcResource::new(
            &self.version[..],
            &self.class_name[..],
            &self.method_name[..],
        );
    }
}