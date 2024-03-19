use std::error::Error;

use super::{grpc_codec::GrpcBodyCodec, json_codec::JsonBodyCodec, BodyCodec, HttpCodec};
use crate::{BoxBody, HttpBody, StreamBody};
use fusen_common::{error::FusenError, net_util::get_uuid, FusenContext, MetaData};

pub struct FusenHttpCodec<D, E> {
    json_codec: Box<dyn BodyCodec<D, E> + Send + 'static>,
    grpc_codec: Box<dyn BodyCodec<D, E> + Send + 'static>,
}

impl<D, E> FusenHttpCodec<D, E> {
    pub fn new() -> Self {
        let json_codec = JsonBodyCodec::<D, E>::new();
        let grpc_codec = GrpcBodyCodec::<D, E>::new();
        FusenHttpCodec {
            json_codec: Box::new(json_codec as dyn BodyCodec<D, E>),
            grpc_codec: Box::new(grpc_codec as dyn BodyCodec<D, E>),
        }
    }
}

impl<D, E> HttpCodec<D, E> for FusenHttpCodec<D, E>
where
    D: bytes::Buf,
    E: Error,
{
    async fn decode(&self, req: http::Request<BoxBody<D, E>>) -> Result<FusenContext, FusenError> {
        let meta_data = MetaData::from(req.headers());
        let codec = match meta_data.get_codec() {
            fusen_common::codec::CodecType::JSON => self.json_codec,
            fusen_common::codec::CodecType::GRPC => self.grpc_codec,
        };
        let msg = codec.decode(req.body()).await?;
        let unique_identifier = meta_data
            .get_value("unique_identifier")
            .map_or(get_uuid(), |e| e);
        let path = req.uri().path().to_string();
        let version = meta_data
            .get_value("tri-service-version")
            .map_or(meta_data.get_value("version"), |e| Some(e))
            .map(|e| e.clone());
        FusenContext::new(unique_identifier, path, meta_data, version, None, None, msg);
    }

    async fn encode(
        &self,
        context: fusen_common::FusenContext,
    ) -> Result<http::Response<StreamBody<D, E>>, FusenError> {
        let meta_data = &context.meta_data;
        let codec = match meta_data.get_codec() {
            fusen_common::codec::CodecType::JSON => self.json_codec,
            fusen_common::codec::CodecType::GRPC => self.grpc_codec,
        };
        let body: Result<StreamBody<HttpBody>, FusenError> = codec.encode(context.res).await;
    }
}
