use std::error::Error;
use std::fmt::Debug;

use crate::BoxBody;
use crate::StreamBody;
use fusen_common::error::FusenError;
use fusen_common::FusenContext;
use http::Request;
use http::Response;
use http_body::Frame;
pub mod grpc_codec;
pub mod http_codec;
pub mod json_codec;

pub trait HttpCodec<D, E>
where
    D: bytes::Buf + Debug,
{
    async fn decode(&self, req: Request<BoxBody<D, E>>) -> Result<FusenContext, FusenError>;

    async fn encode(
        &self,
        context: FusenContext,
    ) -> Result<Response<StreamBody<bytes::Bytes, E>>, FusenError>;
}

pub trait BodyCodec<D, E>
where
    D: bytes::Buf + Debug,
    E: std::error::Error,
{
    fn decode(&self, body: Frame<D>) -> Result<Vec<String>, FusenError>;

    fn encode(
        &self,
        res: Result<String, FusenError>,
    ) -> Result<StreamBody<bytes::Bytes, E>, FusenError>;
}
