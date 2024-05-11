use std::convert::Infallible;

use super::{grpc_codec::GrpcBodyCodec, json_codec::JsonBodyCodec, BodyCodec};
use crate::{support::triple::TripleRequestWrapper, BoxBody};
use bytes::Bytes;
use fusen_common::{
    error::FusenError, logs::get_uuid, register::Type, FusenContext, MetaData, Path,
};
use http::{HeaderValue, Request};
use http_body_util::{BodyExt, Full};

pub(crate) trait RequestCodec<T, E> {
    fn encode(&self, msg: FusenContext) -> Result<Request<BoxBody<T, Infallible>>, crate::Error>;

    async fn decode(&self, request: Request<BoxBody<T, E>>) -> Result<FusenContext, crate::Error>;
}

pub struct RequestHandler {
    json_codec: Box<
        dyn BodyCodec<bytes::Bytes, EncodeType = Vec<String>, DecodeType = Vec<String>>
            + Sync
            + Send,
    >,
    grpc_codec: Box<
        (dyn BodyCodec<
            bytes::Bytes,
            DecodeType = TripleRequestWrapper,
            EncodeType = TripleRequestWrapper,
        > + Sync
             + Send),
    >,
}

impl RequestHandler {
    pub fn new() -> Self {
        let json_codec = JsonBodyCodec::<bytes::Bytes, Vec<String>, Vec<String>>::new();
        let grpc_codec =
            GrpcBodyCodec::<bytes::Bytes, TripleRequestWrapper, TripleRequestWrapper>::new();
        RequestHandler {
            json_codec: Box::new(json_codec),
            grpc_codec: Box::new(grpc_codec),
        }
    }
}

impl Default for RequestHandler {
    fn default() -> Self {
        Self::new()
    }
}

impl RequestCodec<Bytes, hyper::Error> for RequestHandler {
    fn encode(
        &self,
        context: FusenContext,
    ) -> Result<Request<BoxBody<Bytes, Infallible>>, crate::Error> {
        let content_type = match context.server_tyep.as_ref().unwrap().as_ref() {
            &Type::Dubbo => ("application/grpc", "tri-service-version"),
            _ => ("application/json", "version"),
        };
        let mut builder = Request::builder()
            .header("content-type", content_type.0)
            .header("connection", "keep-alive");
        if let Some(version) = &context.context_info.version {
            builder
                .headers_mut()
                .unwrap()
                .insert(content_type.1, HeaderValue::from_str(version).unwrap());
        }
        let path = match context.server_tyep.as_ref().unwrap().as_ref() {
            &Type::SpringCloud => context.context_info.path,
            _ => {
                let path = "/".to_owned() + context.class_name.as_ref() + "/" + &context.method_name;
                match context.path {
                    fusen_common::Path::GET(_) => fusen_common::Path::GET(path),
                    fusen_common::Path::POST(_) => fusen_common::Path::POST(path),
                }
            }
        };
        let request = match path {
            fusen_common::Path::GET(path) => builder
                .method("GET")
                .uri(get_path(path, &msg.fields, &msg.req))
                .body(Full::new(Bytes::new()).boxed()),
            fusen_common::Path::POST(path) => {
                let body: Bytes = match msg.server_tyep.as_ref().unwrap().as_ref() {
                    &Type::Dubbo => {
                        let triple_request_wrapper = TripleRequestWrapper::from(msg.req);
                        self.grpc_codec.encode(triple_request_wrapper)?
                    }
                    _ => self.json_codec.encode(msg.req)?,
                };
                let builder = builder.header("content-length", body.len());
                builder
                    .method("POST")
                    .uri(path)
                    .body(Full::new(body).boxed())
            }
        }?;
        Ok(request)
    }

    async fn decode(
        &self,
        mut request: Request<BoxBody<Bytes, hyper::Error>>,
    ) -> Result<FusenContext, crate::Error> {
        let meta_data = MetaData::from(request.headers());
        let path = request.uri().path().to_string();
        let method = request.method().to_string().to_lowercase();
        let mut frame_vec = vec![];
        let msg = if method.contains("get") {
            let url = request.uri().to_string();
            let url: Vec<&str> = url.split('?').collect();
            let mut vec = vec![];
            if url.len() > 1 {
                let params: Vec<&str> = url[1].split('&').collect();
                for item in params {
                    let item: Vec<&str> = item.split('=').collect();
                    vec.push(item[1].to_owned());
                }
            }
            vec
        } else {
            while let Some(frame) = request.body_mut().frame().await {
                if let Ok(frame) = frame {
                    frame_vec.push(frame);
                }
            }
            if frame_vec.is_empty() {
                return Err(Box::new(FusenError::from("empty frame")));
            }
            let bytes = frame_vec
                .remove(0)
                .into_data()
                .map_or(Err(FusenError::from("empty body")), Ok)?;
            match meta_data.get_codec() {
                fusen_common::codec::CodecType::JSON => {
                    if !bytes.starts_with(b"[") {
                        vec![String::from_utf8_lossy(bytes.as_ref()).to_string()]
                    } else {
                        self.json_codec.decode(&bytes).map_err(FusenError::from)?
                    }
                }
                fusen_common::codec::CodecType::GRPC => self
                    .grpc_codec
                    .decode(&bytes)
                    .map_err(FusenError::from)?
                    .get_req(),
            }
        };
        let unique_identifier = meta_data
            .get_value("unique_identifier")
            .map_or(get_uuid(), |e| e.clone());
        let version = meta_data
            .get_value("tri-service-version")
            .map_or(meta_data.get_value("version"), Some)
            .cloned();
        Ok(FusenContext::new(
            unique_identifier,
            Path::new(&method, path),
            meta_data,
            version,
            None,
            "".to_string(),
            "".to_string(),
            msg,
            vec![],
        ))
    }
}

fn get_path(mut path: String, fields: &[String], msg: &[String]) -> String {
    if !fields.is_empty() {
        path.push('?');
        for idx in 0..fields.len() {
            path.push_str(&fields[idx]);
            path.push('=');
            path.push_str(&msg[idx]);
            path.push('&');
        }
        path.remove(path.len() - 1);
    }
    path
}
