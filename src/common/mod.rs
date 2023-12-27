pub mod date_util;

use std::marker::PhantomData;

use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize)]
pub struct KrpcRequest<Req, Res> {
    #[serde(default)]
    pub req: Req,
    #[serde(default)]
    pub res: Option<Res>,
}

impl<Req, Res> KrpcRequest<Req, Res> {
    pub fn new(req: Req) -> Self {
        return Self { req, res: None };
    }
}

pub struct Resource<Req, Res> {
    _req: PhantomData<Req>,
    _res: PhantomData<Res>,
    version: String,
    class_name: String,
    method_name: String,
}


