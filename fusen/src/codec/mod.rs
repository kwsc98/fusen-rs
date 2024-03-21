use crate::BoxBody;
use crate::StreamBody;
use fusen_common::error::BoxFusenError;
use fusen_common::FusenContext;
use http::Request;
use http::Response;
use http_body::Frame;
use std::fmt::Debug;
pub mod grpc_codec;
pub mod http_codec;
pub mod json_codec;

#[allow(async_fn_in_trait)]
pub trait HttpCodec<D, E>
where
    D: bytes::Buf + Debug,
{
    async fn decode(&self, req: Request<BoxBody<D, E>>) -> Result<FusenContext, BoxFusenError>;

    async fn encode(
        &self,
        context: FusenContext,
    ) -> Result<Response<StreamBody<bytes::Bytes, E>>, BoxFusenError>;
}

pub trait BodyCodec<D, E>
where
    D: bytes::Buf + Debug,
{
    fn decode(&self, body: Vec<Frame<D>>) -> Result<Vec<String>, BoxFusenError>;

    fn encode(
        &self,
        res: Result<String, BoxFusenError>,
    ) -> Result<StreamBody<bytes::Bytes, E>, BoxFusenError>;
}
