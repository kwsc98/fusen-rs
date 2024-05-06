use super::{grpc_codec::GrpcBodyCodec, json_codec::JsonBodyCodec, BodyCodec};
use crate::support::triple::{TripleRequestWrapper, TripleResponseWrapper};
use fusen_common::{codec::CodecType, error::FusenError, FusenContext};
use http::Response;
use http_body_util::BodyExt;
use hyper::body::Incoming;

pub(crate) trait ResponseCodec<T> {
    fn encode(&self, msg: FusenContext) -> Result<Response<T>, crate::Error>;

    async fn decode(&self, request: Response<Incoming>) -> Result<String, FusenError>;
}

pub struct ResponseHandler {
    json_codec: Box<
        dyn BodyCodec<bytes::Bytes, EncodeType = Vec<String>, DecodeType = String> + Sync + Send,
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
        let json_codec: JsonBodyCodec<bytes::Bytes, String, Vec<String>> =
            JsonBodyCodec::<bytes::Bytes, String, Vec<String>>::new();
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

    async fn decode(&self, mut response: Response<Incoming>) -> Result<String, FusenError> {
        if !response.status().is_success() {
            return Err(FusenError::from(format!(
                "err code : {}",
                response.status().as_str()
            )));
        }
        let mut frame_vec = vec![];
        while let Some(body) = response.frame().await {
            if let Ok(body) = body {
                if body.is_trailers() {
                    let trailers = body
                        .trailers_ref()
                        .map_or(Err(FusenError::from("error trailers N1")), |e| Ok(e))?;
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
                                    b"91" => return Err(FusenError::NotFind(msg)),
                                    _ => return Err(FusenError::from(msg)),
                                };
                            }
                        },
                        None => return Err(FusenError::from("error trailers N2")),
                    }
                }
                frame_vec.push(body);
            } else {
                break;
            }
        }
        if frame_vec.is_empty() {
            return Err(FusenError::from("empty frame"));
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
        let byte = frame_vec[0]
            .data_ref()
            .map_or(Err(FusenError::from("empty body")), |e| Ok(e))?;
        let res = match codec_type {
            CodecType::JSON => self.json_codec.decode(byte)?,
            CodecType::GRPC => {
                let response = self.grpc_codec.decode(byte)?;
                String::from_utf8(response.data).map_err(|e| FusenError::from(e.to_string()))?
            }
        };
        Ok(res)
    }
}
