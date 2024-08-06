use std::{convert::Infallible, str::FromStr, sync::Arc};
use super::{grpc_codec::GrpcBodyCodec, json_codec::JsonBodyCodec, BodyCodec};
use crate::{
    filter::server::{PathCache, PathCacheResult},
    support::triple::TripleRequestWrapper,
    BoxBody,
};
use bytes::Bytes;
use fusen_common::{
    error::FusenError, logs::get_uuid, register::Type, ContextInfo, FusenContext, FusenRequest,
    MetaData, Path,
};
use http::{HeaderValue, Request};
use http_body_util::{BodyExt, Full};

pub(crate) trait RequestCodec<T, E> {
    fn encode(&self, msg: &FusenContext) -> Result<Request<BoxBody<T, Infallible>>, crate::Error>;

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
    path_cache: Arc<PathCache>,
}

impl RequestHandler {
    pub fn new(path_cache: Arc<PathCache>) -> Self {
        let json_codec = JsonBodyCodec::<bytes::Bytes, Vec<String>, Vec<String>>::new();
        let grpc_codec =
            GrpcBodyCodec::<bytes::Bytes, TripleRequestWrapper, TripleRequestWrapper>::new();
        RequestHandler {
            json_codec: Box::new(json_codec),
            grpc_codec: Box::new(grpc_codec),
            path_cache,
        }
    }
}

impl RequestCodec<Bytes, hyper::Error> for RequestHandler {
    fn encode(
        &self,
        context: &FusenContext,
    ) -> Result<Request<BoxBody<Bytes, Infallible>>, crate::Error> {
        let content_type = match &context.server_type {
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
        let request = match context.context_info.path.clone() {
            fusen_common::Path::GET(path) => builder
                .method("GET")
                .uri(get_path(
                    path,
                    context.request.fields_ty.as_ref(),
                    &context.request.fields,
                ))
                .body(Full::new(Bytes::new()).boxed()),
            fusen_common::Path::PUT(path) => builder
                .method("PUT")
                .uri(get_path(
                    path,
                    context.request.fields_ty.as_ref(),
                    &context.request.fields,
                ))
                .body(Full::new(Bytes::new()).boxed()),
            fusen_common::Path::DELETE(path) => builder
                .method("DELETE")
                .uri(get_path(
                    path,
                    context.request.fields_ty.as_ref(),
                    &context.request.fields,
                ))
                .body(Full::new(Bytes::new()).boxed()),
            fusen_common::Path::POST(path) => {
                let body: Bytes = match &context.server_type {
                    &Type::Dubbo => {
                        let triple_request_wrapper =
                            TripleRequestWrapper::from(&context.request.fields);
                        self.grpc_codec.encode(&triple_request_wrapper)?
                    }
                    _ => self.json_codec.encode(&context.request.fields)?,
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
        let mut temp_fields_ty = None;
        let mut temp_fields = if method.contains("get") {
            let url = request.uri().to_string();
            let url: Vec<&str> = url.split('?').collect();
            let mut vec = vec![];
            if url.len() > 1 {
                let mut tys = vec![];
                let params: Vec<&str> = url[1].split('&').collect();
                for item in params {
                    let item: Vec<&str> = item.split('=').collect();
                    tys.push(item[0].to_owned());
                    vec.push(item[1].to_owned());
                }
                let _ = temp_fields_ty.insert(tys);
            }
            vec
        } else {
            let mut frame_vec = vec![];
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
        let mut path = Path::new(&method, path);
        let PathCacheResult {
            class,
            method,
            fields,
        } = self
            .path_cache
            .seach(&mut path)
            .ok_or(FusenError::NotFind)?;
        if let Some(mut fields) = fields {
            temp_fields.append(&mut fields.0);
            temp_fields_ty = match temp_fields_ty {
                Some(mut temp_fields_ty) => {
                    temp_fields_ty.append(&mut fields.1);
                    Some(temp_fields_ty)
                }
                None => Some(fields.1),
            }
        }
        let context = FusenContext::new(
            unique_identifier,
            ContextInfo::default()
                .class_name(class)
                .method_name(method)
                .path(path)
                .version(version),
            FusenRequest::new(temp_fields, temp_fields_ty),
            meta_data,
        );
        Ok(context)
    }
}

fn get_path(mut path: String, fields_ty: Option<&Vec<String>>, fields: &[String]) -> String {
    if path.contains('{') {
        return get_rest_path(path, fields_ty, fields);
    }
    if !fields.is_empty() {
        let fields_ty = fields_ty.unwrap();
        path.push('?');
        for idx in 0..fields.len() {
            path.push_str(&fields_ty[idx]);
            path.push('=');
            path.push_str(&fields[idx]);
            path.push('&');
        }
        path.remove(path.len() - 1);
    }
    path
}

fn get_rest_path(mut path: String, fields_ty: Option<&Vec<String>>, fields: &[String]) -> String {
    if !fields.is_empty() {
        let fields_ty = fields_ty.unwrap();
        for idx in 0..fields.len() {
            let mut temp = String::from_str("{").unwrap();
            temp.push_str(&fields_ty[idx]);
            temp.push('}');
            path = path.replace(&temp, &fields[idx].replace('\"', ""));
        }
    }
    path
}
