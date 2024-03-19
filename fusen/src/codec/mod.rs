use std::error::Error;

use crate::BoxBody;
use crate::StreamBody;
use fusen_common::error::FusenError;
use fusen_common::FusenContext;
use http::Request;
use http::Response;
mod grpc_codec;
pub mod http_codec;
mod json_codec;

pub trait HttpCodec<D, E>
where
    D: bytes::Buf,
    E: Error,
{
    async fn decode(&self, req: Request<BoxBody<D, E>>) -> Result<FusenContext, FusenError>;

    async fn encode(&self, context: FusenContext)
        -> Result<Response<StreamBody<D, E>>, FusenError>;
}

pub trait BodyCodec<D, E>
where
    D: bytes::Buf,
    E: Error,
{
    async fn decode(&self, body: BoxBody<D, E>) -> Result<Vec<String>, FusenError>;

    async fn encode(
        &self,
        res: Result<String, FusenError>,
    ) -> Result<StreamBody<bytes::Bytes, E>, FusenError>;
}
