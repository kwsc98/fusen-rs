use crate::BoxBody;
use crate::StreamBody;
use fusen_common::error::FusenError;
use fusen_common::FusenContext;
use http::Request;
use http::Response;
use http_body::Frame;
use std::fmt::Debug;
pub mod grpc_codec;
pub mod http_codec;
pub mod json_codec;
pub mod request_codec;

#[allow(async_fn_in_trait)]
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

pub trait BodyCodec<D>
where
    D: bytes::Buf,
{
    type DecodeType: Send;

    type EncodeType: Send;

    fn decode(&self, body: Vec<Frame<D>>) -> Result<Self::DecodeType, crate::Error>;

    fn encode(&self, res: Self::EncodeType) -> Result<bytes::Bytes, crate::Error>;
}
