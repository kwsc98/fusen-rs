use super::{grpc_codec::GrpcBodyCodec, json_codec::JsonBodyCodec, BodyCodec};
use crate::support::triple::{TripleRequestWrapper, TripleResponseWrapper};
use bytes::Bytes;
use fusen_common::{register::Type, FusenContext};
use http::{HeaderValue, Request};
use http_body_util::Full;

pub(crate) trait RequestCodec<T> {
    fn encode(&self, msg: FusenContext) -> Result<Request<T>, crate::Error>;

    async fn decode(&self, request: Request<T>) -> Result<FusenContext, crate::Error>;
}

pub struct RequestHandler {
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

impl RequestHandler {
    pub fn new() -> Self {
        let json_codec = JsonBodyCodec::<bytes::Bytes>::new();
        let grpc_codec =
            GrpcBodyCodec::<bytes::Bytes, TripleResponseWrapper, TripleRequestWrapper>::new();
        RequestHandler {
            json_codec: Box::new(json_codec),
            grpc_codec: Box::new(grpc_codec),
        }
    }
}

impl RequestCodec<Full<Bytes>> for RequestHandler {
    fn encode(&self, msg: FusenContext) -> Result<Request<Full<Bytes>>, crate::Error> {
        let content_type = match &msg.server_tyep {
            &Type::Dubbo => ("application/grpc", "tri-service-version"),
            _ => ("application/json", "version"),
        };
        let mut builder = Request::builder()
            .header("content-type", content_type.0)
            .header("connection", "keep-alive");
        if let Some(version) = &msg.version {
            builder
                .headers_mut()
                .unwrap()
                .insert(content_type.1, HeaderValue::from_str(&version).unwrap());
        }
        let path = match &msg.server_tyep {
            &Type::SpringCloud => msg.path,
            _ => {
                let path = "/".to_owned() + msg.class_name.as_ref() + "/" + &msg.method_name;
                match msg.path {
                    fusen_common::Path::GET(_) => fusen_common::Path::GET(path),
                    fusen_common::Path::POST(_) => fusen_common::Path::POST(path),
                }
            }
        };
        let body = match msg.server_tyep {
            Type::Dubbo => {
                let triple_request_wrapper = TripleRequestWrapper::from(msg.req);
                self.grpc_codec.encode(triple_request_wrapper)?
            }
            _ => self.json_codec.encode(msg.req)?,
        };
        let builder = builder.header("content-length", body.len());
        let request = match path {
            fusen_common::Path::GET(path) => builder
                .method("GET")
                .uri(get_path(path, &msg.fields, &msg.req))
                .body(Full::new(Bytes::new())),
            fusen_common::Path::POST(path) => {
                builder.method("POST").uri(path).body(Full::new(body))
            }
        }?;
        Ok(request)
    }

    async fn decode(&self, request: Request<Full<Bytes>>) -> Result<FusenContext, crate::Error> {
        todo!()
    }
}

fn get_path(mut path: String, fields: &Vec<String>, msg: &Vec<String>) -> String {
    if fields.len() > 0 {
        path.push_str("?");
        for idx in 0..fields.len() {
            path.push_str(&fields[idx]);
            path.push_str("=");
            path.push_str(&msg[idx]);
            path.push_str("&");
        }
        path.remove(path.len() - 1);
    }
    path
}
