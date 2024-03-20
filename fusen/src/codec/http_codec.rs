use std::{error::Error, fmt::Debug};

use super::{grpc_codec::GrpcBodyCodec, json_codec::JsonBodyCodec, BodyCodec, HttpCodec};
use crate::{BoxBody, HttpBody, StreamBody};
use fusen_common::{error::FusenError, net_util::get_uuid, FusenContext, MetaData};
use http_body_util::BodyExt;
use hyper::body;

pub struct FusenHttpCodec<D, E> {
    json_codec: Box<dyn BodyCodec<D, E> + Sync + Send>,
    grpc_codec: Box<dyn BodyCodec<D, E> + Sync + Send>,
}

impl<D, E> FusenHttpCodec<D, E>
where
    D: bytes::Buf + Debug + Sync + Send + 'static,
    E: std::error::Error + Sync + Send + 'static,
{
    pub fn new() -> Self {
        let json_codec = JsonBodyCodec::<D, E>::new();
        let grpc_codec = GrpcBodyCodec::<D, E>::new();
        FusenHttpCodec {
            json_codec: Box::new(json_codec),
            grpc_codec: Box::new(grpc_codec),
        }
    }
}

impl<D, E> HttpCodec<D, E> for FusenHttpCodec<D, E>
where
    D: bytes::Buf + Debug,
    E: Error,
{
    async fn decode(
        &self,
        mut req: http::Request<BoxBody<D, E>>,
    ) -> Result<FusenContext, FusenError> {
        let meta_data = MetaData::from(req.headers());
        let codec = JsonBodyCodec::<D, E>::new();
        let path = req.uri().path().to_string();
        let body = req.body_mut().frame().await.unwrap().unwrap();
        let msg = codec.decode(body)?;
        let unique_identifier = meta_data
            .get_value("unique_identifier")
            .map_or(get_uuid(), |e| e.clone());
        let version = meta_data
            .get_value("tri-service-version")
            .map_or(meta_data.get_value("version"), |e| Some(e))
            .map(|e| e.clone());
        FusenContext::new(
            unique_identifier,
            path,
            meta_data,
            version,
            "".to_string(),
            "".to_string(),
            msg,
        );
        Err(FusenError::Null)
    }

    async fn encode(
        &self,
        context: fusen_common::FusenContext,
    ) -> Result<http::Response<StreamBody<bytes::Bytes, E>>, FusenError> {
        let meta_data = &context.meta_data;
        let codec = JsonBodyCodec::<D, E>::new();
        let body: Result<StreamBody<bytes::Bytes, E>, FusenError> = codec.encode(context.res);
        Err(FusenError::Null)
    }
}
