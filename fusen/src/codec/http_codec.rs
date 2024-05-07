use super::{grpc_codec::GrpcBodyCodec, json_codec::JsonBodyCodec, BodyCodec, HttpCodec};
use crate::{
    support::triple::{TripleRequestWrapper, TripleResponseWrapper},
    BoxBody, StreamBody,
};
use bytes::Bytes;
use fusen_common::{
    error::FusenError,
    logs::get_uuid,
    FusenContext, MetaData, Path,
};
use http::{HeaderMap, HeaderValue, Response};
use http_body::Frame;
use http_body_util::BodyExt;
use std::fmt::Debug;

pub struct FusenHttpCodec {
    json_codec:
        Box<dyn BodyCodec<Bytes, EncodeType = String, DecodeType = Vec<String>> + Sync + Send>,
    grpc_codec: Box<
        (dyn BodyCodec<Bytes, DecodeType = TripleRequestWrapper, EncodeType = TripleResponseWrapper>
             + Sync
             + Send),
    >,
}

impl FusenHttpCodec {
    pub fn new() -> Self {
        let json_codec = JsonBodyCodec::<Bytes, Vec<String>, String>::new();
        let grpc_codec = GrpcBodyCodec::<Bytes, TripleRequestWrapper, TripleResponseWrapper>::new();
        FusenHttpCodec {
            json_codec: Box::new(json_codec),
            grpc_codec: Box::new(grpc_codec),
        }
    }
}

impl<E> HttpCodec<Bytes, E> for FusenHttpCodec
where
    E: Send + Sync + Debug,
{
    async fn decode(
        &self,
        mut req: http::Request<BoxBody<Bytes, E>>,
    ) -> Result<FusenContext, FusenError> {
       todo!()
    }

    async fn encode(
        &self,
        context: fusen_common::FusenContext,
    ) -> Result<http::Response<StreamBody<bytes::Bytes, E>>, FusenError> {
        todo!()
    }
}
