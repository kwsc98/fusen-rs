use super::{grpc_codec::GrpcBodyCodec, json_codec::JsonBodyCodec, BodyCodec};
use crate::support::triple::{TripleRequestWrapper, TripleResponseWrapper};
use fusen_common::{error::FusenError, FusenContext};
use http::Response;
use hyper::body::Incoming;

pub(crate) trait ResponseCodec<T> {
    fn encode(&self, msg: FusenContext) -> Result<Response<T>, crate::Error>;

    async fn decode(
        &self,
        request: Response<Incoming>,
    ) -> Result<Result<String, FusenError>, crate::Error>;
}

pub struct ResponseHandler {
    json_codec: Box<
        dyn BodyCodec<bytes::Bytes, EncodeType = Vec<String>, DecodeType = Vec<String>>
            + Sync
            + Send,
    >,
    grpc_codec: Box<
        (dyn BodyCodec<
            bytes::Bytes,
            DecodeType = TripleResponseWrapper,
            EncodeType = TripleRequestWrapper,
        > + Sync
             + Send),
    >,
}

impl ResponseHandler {
    pub fn new() -> Self {
        let json_codec = JsonBodyCodec::<bytes::Bytes>::new();
        let grpc_codec =
            GrpcBodyCodec::<bytes::Bytes, TripleResponseWrapper, TripleRequestWrapper>::new();
        ResponseHandler {
            json_codec: Box::new(json_codec),
            grpc_codec: Box::new(grpc_codec),
        }
    }
}

impl ResponseCodec<Incoming> for ResponseHandler {
    fn encode(&self, mut msg: FusenContext) -> Result<Response<Incoming>, crate::Error> {
        todo!()
    }

    async fn decode(
        &self,
        request: Response<Incoming>,
    ) -> Result<Result<String, FusenError>, crate::Error> {
        todo!()
    }
}
