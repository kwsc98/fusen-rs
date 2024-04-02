use super::{grpc_codec::GrpcBodyCodec, json_codec::JsonBodyCodec, BodyCodec, HttpCodec};
use crate::{BoxBody, StreamBody};
use fusen_common::{error::FusenError, logs::get_uuid, FusenContext, MetaData};
use http::Response;
use http_body_util::BodyExt;
use std::fmt::Debug;

pub struct FusenHttpCodec<D, E> {
    json_codec: Box<dyn BodyCodec<D, E> + Sync + Send>,
    grpc_codec: Box<dyn BodyCodec<D, E> + Sync + Send>,
}

impl<D, E> FusenHttpCodec<D, E>
where
    D: bytes::Buf + Debug + Sync + Send + 'static,
    E: std::marker::Sync + std::marker::Send + 'static,
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
    E: Send + Sync + Debug,
{
    async fn decode(
        &self,
        mut req: http::Request<BoxBody<D, E>>,
    ) -> Result<FusenContext, FusenError> {
        let meta_data = MetaData::from(req.headers());
        let codec = match meta_data.get_codec() {
            fusen_common::codec::CodecType::JSON => &self.json_codec,
            fusen_common::codec::CodecType::GRPC => &self.grpc_codec,
        };
        let path = req.uri().path().to_string();
        let mut frame_vec = vec![];
        while let Some(frame) = req.body_mut().frame().await {
            if let Ok(frame) = frame {
                frame_vec.push(frame);
            }
        }
        let msg = codec.decode(frame_vec)?;
        let unique_identifier = meta_data
            .get_value("unique_identifier")
            .map_or(get_uuid(), |e| e.clone());
        let version = meta_data
            .get_value("tri-service-version")
            .map_or(meta_data.get_value("version"), |e| Some(e))
            .map(|e| e.clone());
        Ok(FusenContext::new(
            unique_identifier,
            path,
            meta_data,
            version,
            None,
            "".to_string(),
            "".to_string(),
            msg,
        ))
    }

    async fn encode(
        &self,
        context: fusen_common::FusenContext,
    ) -> Result<http::Response<StreamBody<bytes::Bytes, E>>, FusenError> {
        let meta_data = &context.meta_data;
        let codec = match meta_data.get_codec() {
            fusen_common::codec::CodecType::JSON => &self.json_codec,
            fusen_common::codec::CodecType::GRPC => &self.grpc_codec,
        };
        let content_type = match meta_data.get_codec() {
            fusen_common::codec::CodecType::JSON => "application/json",
            fusen_common::codec::CodecType::GRPC => "application/grpc",
        };
        let body = codec.encode(context.res)?;
        let response = Response::builder()
            .header("content-type", content_type)
            .body(body)
            .map_err(|e| FusenError::Server(e.to_string()))?;
        Ok(response)
    }
}
