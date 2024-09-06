use super::{grpc_codec::GrpcBodyCodec, BodyCodec};
use crate::{
    filter::server::{PathCache, PathCacheResult},
    support::triple::TripleRequestWrapper,
    BoxBody,
};
use bytes::{Bytes, BytesMut};
use fusen_common::{
    error::FusenError, logs::get_trade_id, register::Type, ContextInfo, FusenContext, FusenRequest,
    MetaData, Path,
};
use http::Request;
use http_body_util::{BodyExt, Full};
use std::{convert::Infallible, sync::Arc};

pub(crate) trait RequestCodec<T, E> {
    fn encode(&self, msg: &FusenContext) -> Result<Request<BoxBody<T, Infallible>>, crate::Error>;

    async fn decode(&self, request: Request<BoxBody<T, E>>) -> Result<FusenContext, crate::Error>;
}

pub struct RequestHandler {
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
        let grpc_codec =
            GrpcBodyCodec::<bytes::Bytes, TripleRequestWrapper, TripleRequestWrapper>::new();
        RequestHandler {
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
        let content_type = match context.get_server_type() {
            &Type::Dubbo => ("application/grpc", "tri-service-version"),
            _ => ("application/json", "version"),
        };
        let mut builder = Request::builder()
            .header("content-type", content_type.0)
            .header("connection", "keep-alive");
        for (key, value) in context.get_request().get_headers() {
            builder = builder.header(key, value);
        }
        if let Some(version) = context.get_context_info().get_version() {
            builder = builder.header(content_type.1, version);
        }
        let request = match context.get_context_info().get_path().clone() {
            fusen_common::Path::GET(path) => builder
                .method("GET")
                .uri(get_path(
                    path,
                    context.get_request().get_query_fields().as_ref(),
                ))
                .body(Full::new(Bytes::new()).boxed()),
            fusen_common::Path::PUT(path) => builder
                .method("PUT")
                .uri(get_path(
                    path,
                    context.get_request().get_query_fields().as_ref(),
                ))
                .body(Full::new(Bytes::new()).boxed()),
            fusen_common::Path::DELETE(path) => builder
                .method("DELETE")
                .uri(get_path(
                    path,
                    context.get_request().get_query_fields().as_ref(),
                ))
                .body(Full::new(Bytes::new()).boxed()),
            fusen_common::Path::POST(mut path) => {
                let body: Bytes = match context.get_server_type() {
                    &Type::Dubbo => {
                        path = format!(
                            "/{}/{}",
                            context.get_context_info().get_class_name(),
                            context.get_context_info().get_method_name()
                        );
                        let fields: Vec<String> =
                            serde_json::from_slice(context.get_request().get_body())?;
                        let triple_request_wrapper = TripleRequestWrapper::from(fields);
                        self.grpc_codec.encode(&triple_request_wrapper)?
                    }
                    _ => Bytes::copy_from_slice(context.get_request().get_body()),
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
        let mut temp_query_fields_ty: Vec<(String, String)> = vec![];
        let mut body = BytesMut::new();
        if method.contains("get") {
            let url = request.uri().to_string();
            let url: Vec<&str> = url.split('?').collect();
            if url.len() > 1 {
                let params: Vec<&str> = url[1].split('&').collect();
                for item in params {
                    let item: Vec<&str> = item.split('=').collect();
                    temp_query_fields_ty.push((item[0].to_owned(), item[1].to_owned()));
                }
            }
        } else {
            let mut bytes = BytesMut::new();
            while let Some(Ok(frame)) = request.body_mut().frame().await {
                if frame.is_data() {
                    bytes.extend(frame.into_data().unwrap());
                }
            }
            let bytes: Bytes = bytes.into();
            match meta_data.get_codec() {
                fusen_common::codec::CodecType::JSON => {
                    body.extend_from_slice(&bytes);
                }
                fusen_common::codec::CodecType::GRPC => {
                    let bytes = self
                        .grpc_codec
                        .decode(&bytes)
                        .map_err(FusenError::from)?
                        .get_body();
                    body.extend_from_slice(&bytes);
                }
            }
        };
        let unique_identifier = meta_data
            .get_value("unique_identifier")
            .map_or(get_trade_id(), |e| e.clone());
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
            temp_query_fields_ty.append(&mut fields);
        }
        let context = FusenContext::new(
            unique_identifier,
            ContextInfo::default()
                .class_name(class)
                .method_name(method)
                .path(path)
                .version(version),
            FusenRequest::new(
                if temp_query_fields_ty.is_empty() {
                    None
                } else {
                    Some(temp_query_fields_ty)
                },
                body.into(),
            ),
            meta_data,
        );
        Ok(context)
    }
}

fn get_path(mut path: String, query_fields: Option<&Vec<(String, String)>>) -> String {
    if path.contains('{') {
        return get_rest_path(path, query_fields);
    }
    if let Some(query_fields) = query_fields {
        path.push('?');
        for item in query_fields {
            path.push_str(&item.0);
            path.push('=');
            path.push_str(&item.1);
            path.push('&');
        }
        path.remove(path.len() - 1);
    }
    path
}

fn get_rest_path(mut path: String, query_fields: Option<&Vec<(String, String)>>) -> String {
    if let Some(query_fields) = query_fields {
        for item in query_fields {
            let temp = format!("{{{}}}", item.0);
            path = path.replace(&temp, &item.1.replace('\"', ""));
        }
    }
    path
}
