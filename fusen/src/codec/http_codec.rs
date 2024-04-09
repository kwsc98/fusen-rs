use super::{grpc_codec::GrpcBodyCodec, json_codec::JsonBodyCodec, BodyCodec, HttpCodec};
use crate::{
    support::triple::{TripleRequestWrapper, TripleResponseWrapper},
    BoxBody, StreamBody,
};
use fusen_common::{error::FusenError, logs::get_uuid, FusenContext, MetaData, Path};
use http::{HeaderMap, HeaderValue, Response};
use http_body::Frame;
use http_body_util::BodyExt;
use std::fmt::Debug;

pub struct FusenHttpCodec<D> {
    json_codec:
        Box<dyn BodyCodec<D, EncodeType = Vec<String>, DecodeType = Vec<String>> + Sync + Send>,
    grpc_codec: Box<
        (dyn BodyCodec<D, DecodeType = TripleRequestWrapper, EncodeType = TripleResponseWrapper>
             + Sync
             + Send),
    >,
}

impl<D> FusenHttpCodec<D>
where
    D: bytes::Buf + Debug + Sync + Send + 'static,
{
    pub fn new() -> Self {
        let json_codec = JsonBodyCodec::<D>::new();
        let grpc_codec = GrpcBodyCodec::<D, TripleRequestWrapper, TripleResponseWrapper>::new();
        FusenHttpCodec {
            json_codec: Box::new(json_codec),
            grpc_codec: Box::new(grpc_codec),
        }
    }
}

impl<D, E> HttpCodec<D, E> for FusenHttpCodec<D>
where
    D: bytes::Buf + Debug,
    E: Send + Sync + Debug,
{
    async fn decode(
        &self,
        mut req: http::Request<BoxBody<D, E>>,
    ) -> Result<FusenContext, FusenError> {
        let meta_data = MetaData::from(req.headers());
        let path = req.uri().path().to_string();
        let method = req.method().to_string().to_lowercase();
        let mut frame_vec = vec![];
        let msg = if method.contains("get") {
            let url = req.uri().to_string();
            let url: Vec<&str> = url.split("?").collect();
            let mut vec = vec![];
            if url.len() > 1 {
                let params: Vec<&str> = url[1].split("&").collect();
                for item in params {
                    let item: Vec<&str> = item.split("=").collect();
                    vec.push(item[1].to_owned());
                }
            }
            vec
        } else {
            while let Some(frame) = req.body_mut().frame().await {
                if let Ok(frame) = frame {
                    frame_vec.push(frame);
                }
            }
            match meta_data.get_codec() {
                fusen_common::codec::CodecType::JSON => self
                    .json_codec
                    .decode(frame_vec)
                    .map_err(|e| FusenError::Server(e.to_string()))?,
                fusen_common::codec::CodecType::GRPC => self
                    .grpc_codec
                    .decode(frame_vec)
                    .map_err(|e| FusenError::Server(e.to_string()))?
                    .get_req(),
            }
        };
        let unique_identifier = meta_data
            .get_value("unique_identifier")
            .map_or(get_uuid(), |e| e.clone());
        let version = meta_data
            .get_value("tri-service-version")
            .map_or(meta_data.get_value("version"), |e| Some(e))
            .map(|e| e.clone());
        Ok(FusenContext::new(
            unique_identifier,
            Path::new(&method, path),
            meta_data,
            version,
            None,
            "".to_string(),
            "".to_string(),
            msg,
            vec![],
        ))
    }

    async fn encode(
        &self,
        context: fusen_common::FusenContext,
    ) -> Result<http::Response<StreamBody<bytes::Bytes, E>>, FusenError> {
        let meta_data = &context.meta_data;
        let content_type = match meta_data.get_codec() {
            fusen_common::codec::CodecType::JSON => "application/json",
            fusen_common::codec::CodecType::GRPC => "application/grpc",
        };
        let body = match meta_data.get_codec() {
            fusen_common::codec::CodecType::JSON => vec![match context.res {
                Ok(res) => Frame::data(
                    self.json_codec
                        .encode(vec![res])
                        .map_err(|e| FusenError::Server(e.to_string()))?,
                ),
                Err(err) => {
                    if let FusenError::Null = err {
                        Frame::data(bytes::Bytes::from("null"))
                    } else {
                        return Err(err);
                    }
                }
            }],
            fusen_common::codec::CodecType::GRPC => {
                let mut status = "0";
                let mut message = String::from("success");
                let mut trailers = HeaderMap::new();
                let mut vec = vec![];
                match context.res {
                    Ok(data) => {
                        let res_wrapper = TripleResponseWrapper::form(data);
                        let buf = self
                            .grpc_codec
                            .encode(res_wrapper)
                            .map_err(|e| FusenError::Server(e.to_string()))?;
                        vec.push(Frame::data(buf));
                    }
                    Err(err) => {
                        message = match err {
                            FusenError::Client(msg) => {
                                status = "90";
                                msg
                            }
                            FusenError::Method(msg) => {
                                status = "91";
                                msg
                            }
                            FusenError::Null => {
                                status = "92";
                                "FusenError::Null".to_string()
                            }
                            FusenError::ResourceEmpty(msg) => {
                                status = "93";
                                msg
                            }
                            FusenError::Server(msg) => {
                                status = "94";
                                msg
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
            .body(stream_body)
            .map_err(|e| FusenError::Server(e.to_string()))?;
        Ok(response)
    }
}
