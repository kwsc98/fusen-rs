use std::convert::Infallible;

use crate::BoxBody;
use fusen_common::FusenContext;
use http::Request;
use http::Response;
pub mod grpc_codec;
pub mod http_codec;
pub mod json_codec;
pub mod request_codec;
pub mod response_codec;

#[allow(async_fn_in_trait)]
pub trait HttpCodec<D, E>
where
    D: bytes::Buf,
{
    async fn decode(&self, req: Request<BoxBody<D, E>>) -> Result<FusenContext, crate::Error>;

    async fn encode(
        &self,
        context: FusenContext,
    ) -> Result<Response<BoxBody<D, Infallible>>, crate::Error>;
}

pub trait BodyCodec<D>
where
    D: bytes::Buf,
{
    type DecodeType;

    type EncodeType;

    fn decode(&self, body: &D) -> Result<Self::DecodeType, crate::Error>;

    fn encode(&self, res: Self::EncodeType) -> Result<bytes::Bytes, crate::Error>;
}
