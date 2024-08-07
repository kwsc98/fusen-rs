use super::{grpc_codec::GrpcBodyCodec, json_codec::JsonBodyCodec, BodyCodec};
use crate::support::triple::TripleResponseWrapper;
use bytes::{Bytes, BytesMut};
use fusen_common::{codec::CodecType, error::FusenError, FusenContext};
use http::{HeaderMap, HeaderValue, Response};
use http_body::Frame;
use http_body_util::{combinators::BoxBody, BodyExt};
use std::convert::Infallible;

pub(crate) trait ResponseCodec<T, E> {
    fn encode(&self, msg: &FusenContext) -> Result<Response<BoxBody<T, Infallible>>, crate::Error>;

    async fn decode(&self, request: Response<BoxBody<T, E>>) -> Result<String, FusenError>;
}

pub struct ResponseHandler {
    json_codec:
        Box<dyn BodyCodec<bytes::Bytes, EncodeType = String, DecodeType = String> + Sync + Send>,
    grpc_codec: Box<
        (dyn BodyCodec<
            bytes::Bytes,
            DecodeType = TripleResponseWrapper,
            EncodeType = TripleResponseWrapper,
        > + Sync
             + Send),
    >,
}

impl ResponseHandler {
    pub fn new() -> Self {
        let json_codec: JsonBodyCodec<bytes::Bytes, String, String> =
            JsonBodyCodec::<bytes::Bytes, String, String>::new();
        let grpc_codec =
            GrpcBodyCodec::<bytes::Bytes, TripleResponseWrapper, TripleResponseWrapper>::new();
        ResponseHandler {
            json_codec: Box::new(json_codec),
            grpc_codec: Box::new(grpc_codec),
        }
    }
}

impl Default for ResponseHandler {
    fn default() -> Self {
        Self::new()
    }
}

impl ResponseCodec<Bytes, hyper::Error> for ResponseHandler {
    fn encode(
        &self,
        context: &FusenContext,
    ) -> Result<Response<BoxBody<Bytes, Infallible>>, crate::Error> {
        let meta_data = &context.meta_data;
        let content_type = match meta_data.get_codec() {
            fusen_common::codec::CodecType::JSON => "application/json",
            fusen_common::codec::CodecType::GRPC => "application/grpc",
        };
        let body = match meta_data.get_codec() {
            fusen_common::codec::CodecType::JSON => vec![match &context.response.response {
                Ok(res) => Frame::data(self.json_codec.encode(res).map_err(FusenError::from)?),
                Err(err) => {
                    if let FusenError::Null = err {
                        Frame::data(bytes::Bytes::from("null"))
                    } else {
                        return Err(crate::Error::from(err.to_string()));
                    }
                }
            }],
            fusen_common::codec::CodecType::GRPC => {
                let mut status = "0";
                let mut message = String::from("success");
                let mut trailers = HeaderMap::new();
                let mut vec = vec![];
                match context.response.response.as_ref() {
                    Ok(data) => {
                        let res_wrapper = TripleResponseWrapper::form(data.as_bytes());
                        let buf = self
                            .grpc_codec
                            .encode(&res_wrapper)
                            .map_err(FusenError::from)?;
                        vec.push(Frame::data(buf));
                    }
                    Err(err) => {
                        message = match err {
                            FusenError::Null => {
                                let res_wrapper = TripleResponseWrapper::form("null".as_bytes());
                                let buf = self
                                    .grpc_codec
                                    .encode(&res_wrapper)
                                    .map_err(FusenError::from)?;
                                vec.push(Frame::data(buf));
                                "null value".to_owned()
                            }
                            FusenError::NotFind => {
                                status = "91";
                                "not find".to_owned()
                            }
                            FusenError::Info(msg) => {
                                status = "92";
                                msg.clone()
                            }
                        }
                    }
                }
                trailers.insert("grpc-status", HeaderValue::from_str(status).unwrap());
                trailers.insert("grpc-message", HeaderValue::from_str(&message).unwrap());
                vec.push(Frame::trailers(trailers));
                vec
            }
        };

        let chunks = body.into_iter().fold(vec![], |mut vec, e| {
            vec.push(Ok(e));
            vec
        });
        let stream = futures_util::stream::iter(chunks);
        let stream_body = http_body_util::StreamBody::new(stream);
        let response = Response::builder()
            .header("content-type", content_type)
            .body(stream_body.boxed())
            .map_err(FusenError::from)?;
        Ok(response)
    }

    async fn decode(
        &self,
        mut response: Response<BoxBody<Bytes, hyper::Error>>,
    ) -> Result<String, FusenError> {
        if !response.status().is_success() {
            return Err(FusenError::from(format!(
                "err code : {}",
                response.status().as_str()
            )));
        }
        let mut bytes = BytesMut::new();
        while let Some(Ok(frame)) = response.frame().await {
            if frame.is_trailers() {
                let trailers = frame
                    .trailers_ref()
                    .ok_or(FusenError::from("error trailers N1"))?;
                match trailers.get("grpc-status") {
                    Some(status) => match status.as_bytes() {
                        b"0" => {
                            break;
                        }
                        else_status => {
                            let msg = match trailers.get("grpc-message") {
                                Some(value) => {
                                    String::from_utf8(value.as_bytes().to_vec()).unwrap()
                                }
                                None => {
                                    "grpc-status=".to_owned()
                                        + &String::from_utf8(else_status.to_vec()).unwrap()
                                }
                            };
                            match else_status {
                                b"90" => return Err(FusenError::Null),
                                b"91" => return Err(FusenError::NotFind),
                                _ => return Err(FusenError::from(msg)),
                            };
                        }
                    },
                    None => return Err(FusenError::from("error trailers N2")),
                }
            }
            bytes.extend(frame.into_data().unwrap());
        }
        let codec_type = response
            .headers()
            .iter()
            .find(|e| e.0.as_str().to_lowercase() == "content-type")
            .map(|e| e.1)
            .map_or(CodecType::JSON, |e| match e.to_str() {
                Ok(coder) => CodecType::from(coder),
                Err(_) => CodecType::JSON,
            });
        let bytes: Bytes = bytes.into();
        let res = match codec_type {
            CodecType::JSON => {
                if !bytes.to_ascii_lowercase().starts_with(b"\"") {
                    String::from_utf8(bytes.to_vec())
                        .map_err(|e| FusenError::from(e.to_string()))?
                } else {
                    self.json_codec
                        .decode(&bytes)
                        .map_err(|e| FusenError::from(e.to_string()))?
                }
            }
            CodecType::GRPC => {
                let response = self.grpc_codec.decode(&bytes)?;
                String::from_utf8(response.data).map_err(|e| FusenError::from(e.to_string()))?
            }
        };
        Ok(res)
    }
}
