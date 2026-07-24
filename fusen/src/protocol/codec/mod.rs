use crate::{
    error::FusenError,
    protocol::{
        codec::body::{RequestBodyCodec, ResponseBodyCodec, json::JsonCodec},
        fusen::{
            request::{FusenRequest, Path, QueryParameters},
            response::RpcResponse,
        },
    },
};
use bytes::{Bytes, BytesMut};
use fusen_contract::WireProtocol;
use http::{Request, Response, Uri, Version, header::CONTENT_TYPE};
use http_body_util::{BodyExt, Full, combinators::BoxBody};
use hyper::body::Body;
use mime::Mime;
use std::{convert::Infallible, str::FromStr};

pub mod body;

pub const DEFAULT_MAX_BODY_BYTES: usize = 2 * 1024 * 1024;

/// Internal HTTP codec shared by the client and server transports.
pub struct FusenHttpCodec {
    json_codec: JsonCodec,
    max_body_bytes: usize,
}

impl Default for FusenHttpCodec {
    fn default() -> Self {
        Self::new(DEFAULT_MAX_BODY_BYTES)
    }
}

impl FusenHttpCodec {
    /// Creates a codec with a hard limit for decoded request and response bodies.
    pub fn new(max_body_bytes: usize) -> Self {
        Self {
            json_codec: JsonCodec,
            max_body_bytes,
        }
    }
}

impl RequestCodec<Bytes, hyper::Error> for FusenHttpCodec {
    fn encode(
        &self,
        fusen_request: &mut FusenRequest,
    ) -> Result<Request<BoxBody<Bytes, Infallible>>, FusenError> {
        let uri = build_uri(fusen_request)?;
        let body = match fusen_request.body.take() {
            Some(body) => RequestBodyCodec::encode(&self.json_codec, body)?,
            None => Bytes::new(),
        };
        let version = match fusen_request.protocol {
            WireProtocol::Fusen => Version::HTTP_2,
            WireProtocol::SpringCloud => Version::HTTP_11,
            _ => {
                return Err(FusenError::UnsupportedProtocol(
                    fusen_request.protocol.to_string(),
                ));
            }
        };
        let has_body = !body.is_empty();
        let mut request = Request::builder()
            .version(version)
            .method(fusen_request.path.method.clone())
            .uri(uri)
            .body(Full::new(body).boxed())
            .map_err(|error| FusenError::internal("failed to build HTTP request", error))?;
        *request.headers_mut() = std::mem::take(&mut fusen_request.headers);
        if has_body {
            request.headers_mut().insert(
                CONTENT_TYPE,
                http::HeaderValue::from_static("application/json"),
            );
        }
        Ok(request)
    }

    async fn decode(
        &self,
        mut request: Request<BoxBody<Bytes, hyper::Error>>,
    ) -> Result<FusenRequest, FusenError> {
        let query_parameters = request.uri().query().map(parse_query).unwrap_or_default();
        let headers = std::mem::take(request.headers_mut());
        let bytes = read_body(request.body_mut(), self.max_body_bytes, false).await?;
        let content_type = parse_content_type(&headers, false)?;
        if let Some(content_type) = &content_type
            && !is_json(content_type)
        {
            let message = if is_grpc(content_type) {
                "Dubbo Triple is disabled in 0.9".into()
            } else {
                format!("unsupported request content type {content_type}")
            };
            return Err(FusenError::UnsupportedProtocol(message));
        }
        let body = if bytes.is_empty() {
            None
        } else if content_type.is_some() {
            Some(RequestBodyCodec::decode(&self.json_codec, bytes)?)
        } else {
            return Err(FusenError::UnsupportedProtocol(
                "a non-empty request body requires application/json".into(),
            ));
        };
        Ok(FusenRequest {
            path: Path {
                method: request.method().clone(),
                path: request.uri().path().to_owned(),
            },
            endpoint: None,
            path_parameters: Default::default(),
            query_parameters,
            headers,
            body,
            protocol: if request.version() == Version::HTTP_2 {
                WireProtocol::Fusen
            } else {
                WireProtocol::SpringCloud
            },
        })
    }
}

impl ResponseCodec<Bytes, hyper::Error> for FusenHttpCodec {
    fn encode(
        &self,
        fusen_response: &mut RpcResponse,
    ) -> Result<Response<BoxBody<Bytes, Infallible>>, FusenError> {
        let body = match fusen_response.body.take() {
            Some(value) => ResponseBodyCodec::encode(&self.json_codec, value)?,
            None => Bytes::new(),
        };
        let has_body = !body.is_empty();
        let mut response = Response::builder()
            .status(fusen_response.status)
            .body(Full::new(body).boxed())
            .map_err(|error| FusenError::internal("failed to build HTTP response", error))?;
        *response.headers_mut() = std::mem::take(&mut fusen_response.headers);
        if has_body {
            response.headers_mut().insert(
                CONTENT_TYPE,
                http::HeaderValue::from_static("application/json"),
            );
        }
        Ok(response)
    }

    async fn decode(
        &self,
        mut response: Response<BoxBody<Bytes, hyper::Error>>,
    ) -> Result<RpcResponse, FusenError> {
        let headers = std::mem::take(response.headers_mut());
        let bytes = read_body(response.body_mut(), self.max_body_bytes, true).await?;
        let content_type = parse_content_type(&headers, true)?;
        if let Some(content_type) = &content_type
            && !is_json(content_type)
        {
            return Err(FusenError::InvalidResponse(format!(
                "unsupported response content type {content_type}"
            )));
        }
        let body = if bytes.is_empty() {
            None
        } else if content_type.is_some() {
            Some(ResponseBodyCodec::decode(&self.json_codec, bytes)?)
        } else {
            return Err(FusenError::InvalidResponse(
                "a non-empty response body requires a JSON content type".into(),
            ));
        };
        Ok(RpcResponse {
            protocol: if response.version() == Version::HTTP_2 {
                WireProtocol::Fusen
            } else {
                WireProtocol::SpringCloud
            },
            status: response.status(),
            headers,
            body,
        })
    }
}

fn build_uri(request: &mut FusenRequest) -> Result<Uri, FusenError> {
    let endpoint = request
        .endpoint
        .as_ref()
        .ok_or_else(|| FusenError::ServiceUnavailable("request endpoint is missing".into()))?;
    let mut url = endpoint.as_url().clone();
    {
        let mut segments = url.path_segments_mut().map_err(|_| {
            FusenError::InvalidRequest("request endpoint cannot be a base URL".into())
        })?;
        segments.pop_if_empty();
        for segment in request.path.path.trim_matches('/').split('/') {
            if segment.is_empty() {
                continue;
            }
            if let Some(name) = segment
                .strip_prefix('{')
                .and_then(|value| value.strip_suffix('}'))
            {
                let value = request.path_parameters.remove(name).ok_or_else(|| {
                    FusenError::InvalidRequest(format!("missing path parameter {name}"))
                })?;
                segments.push(&value);
            } else {
                segments.push(segment);
            }
        }
    }
    if let Some(name) = request.path_parameters.keys().next() {
        return Err(FusenError::InvalidRequest(format!(
            "unused path parameter {name}"
        )));
    }
    if request
        .query_parameters
        .values()
        .any(|values| !values.is_empty())
    {
        let mut pairs = url.query_pairs_mut();
        for (name, values) in &request.query_parameters {
            for value in values {
                pairs.append_pair(name, value);
            }
        }
    }
    Uri::from_str(url.as_str()).map_err(|error| FusenError::InvalidRequest(error.to_string()))
}

fn parse_query(query: &str) -> QueryParameters {
    let mut parameters = QueryParameters::new();
    for (name, value) in url::form_urlencoded::parse(query.as_bytes()) {
        parameters
            .entry(name.into_owned())
            .or_default()
            .push(value.into_owned());
    }
    parameters
}

fn parse_content_type(
    headers: &http::HeaderMap,
    response: bool,
) -> Result<Option<Mime>, FusenError> {
    let mut values = headers.get_all(CONTENT_TYPE).iter();
    let Some(value) = values.next() else {
        return Ok(None);
    };
    if values.next().is_some() {
        return Err(content_type_error(
            response,
            "multiple Content-Type headers are not allowed".into(),
        ));
    }
    let value = value.to_str().map_err(|error| {
        content_type_error(response, format!("invalid Content-Type header: {error}"))
    })?;
    value.parse::<Mime>().map(Some).map_err(|error| {
        content_type_error(response, format!("invalid Content-Type header: {error}"))
    })
}

fn content_type_error(response: bool, message: String) -> FusenError {
    if response {
        FusenError::InvalidResponse(message)
    } else {
        FusenError::UnsupportedProtocol(message)
    }
}

fn is_json(value: &Mime) -> bool {
    (value.type_() == mime::APPLICATION && value.subtype() == mime::JSON)
        || value.essence_str() == "application/problem+json"
}

fn is_grpc(value: &Mime) -> bool {
    value.type_() == mime::APPLICATION && value.subtype().as_str().starts_with("grpc")
}

#[allow(async_fn_in_trait)]
/// Internal request encoder and decoder contract used by HTTP transports.
pub trait RequestCodec<T, E> {
    /// Encodes a typed RPC request into an HTTP request.
    fn encode(
        &self,
        request: &mut FusenRequest,
    ) -> Result<Request<BoxBody<T, Infallible>>, FusenError>;
    /// Decodes an HTTP request into the framework request representation.
    async fn decode(&self, request: Request<BoxBody<T, E>>) -> Result<FusenRequest, FusenError>;
}

#[allow(async_fn_in_trait)]
pub trait ResponseCodec<T, E> {
    fn encode(
        &self,
        response: &mut RpcResponse,
    ) -> Result<Response<BoxBody<T, Infallible>>, FusenError>;
    async fn decode(&self, response: Response<BoxBody<T, E>>) -> Result<RpcResponse, FusenError>;
}

async fn read_body(
    body: &mut BoxBody<Bytes, hyper::Error>,
    limit: usize,
    response: bool,
) -> Result<Bytes, FusenError> {
    let size_hint = body.size_hint();
    let estimated = size_hint
        .upper()
        .unwrap_or_else(|| size_hint.lower())
        .min(limit as u64) as usize;
    let mut bytes = BytesMut::with_capacity(estimated);
    while let Some(frame) = body.frame().await {
        let frame = frame.map_err(|error| {
            if response {
                FusenError::InvalidResponse(error.to_string())
            } else {
                FusenError::InvalidRequest(error.to_string())
            }
        })?;
        if let Ok(data) = frame.into_data() {
            if bytes.len().saturating_add(data.len()) > limit {
                return Err(FusenError::PayloadTooLarge { limit });
            }
            bytes.extend_from_slice(&data);
        }
    }
    Ok(bytes.freeze())
}

#[cfg(test)]
mod tests {
    use super::*;
    use http::{HeaderValue, Method, header::SET_COOKIE};
    use std::collections::HashMap;

    fn body(value: &'static [u8]) -> BoxBody<Bytes, hyper::Error> {
        Full::new(Bytes::from_static(value))
            .map_err(|never: Infallible| -> hyper::Error { match never {} })
            .boxed()
    }

    #[tokio::test]
    async fn rejects_oversized_body_regardless_of_content_type() {
        let codec = FusenHttpCodec::new(3);
        let request = Request::builder()
            .method("POST")
            .uri("/demo")
            .header(CONTENT_TYPE, "application/octet-stream")
            .body(body(b"1234"))
            .unwrap();
        let error = RequestCodec::decode(&codec, request).await.unwrap_err();
        assert!(matches!(error, FusenError::PayloadTooLarge { limit: 3 }));
    }

    #[tokio::test]
    async fn body_limit_precedes_malformed_content_type_rejection() {
        let codec = FusenHttpCodec::new(3);
        let request = Request::builder()
            .method("POST")
            .uri("/demo")
            .header(CONTENT_TYPE, "not a mime")
            .body(body(b"1234"))
            .unwrap();
        let error = RequestCodec::decode(&codec, request).await.unwrap_err();
        assert!(matches!(error, FusenError::PayloadTooLarge { limit: 3 }));
    }

    #[tokio::test]
    async fn rejects_grpc_content_type() {
        let codec = FusenHttpCodec::default();
        let request = Request::builder()
            .method("POST")
            .uri("/demo")
            .header(CONTENT_TYPE, "application/grpc")
            .body(body(b""))
            .unwrap();
        let error = RequestCodec::decode(&codec, request).await.unwrap_err();
        assert!(matches!(error, FusenError::UnsupportedProtocol(_)));
    }

    #[tokio::test]
    async fn rejects_content_type_prefix_lookalikes() {
        let codec = FusenHttpCodec::default();
        let request = Request::builder()
            .method("POST")
            .uri("/demo")
            .header(CONTENT_TYPE, "application/json-malformed")
            .body(body(b"{}"))
            .unwrap();
        let error = RequestCodec::decode(&codec, request).await.unwrap_err();
        assert!(matches!(error, FusenError::UnsupportedProtocol(_)));
    }

    #[tokio::test]
    async fn rejects_duplicate_request_content_type() {
        let codec = FusenHttpCodec::default();
        let mut request = Request::builder()
            .method("POST")
            .uri("/demo")
            .body(body(b"{}"))
            .unwrap();
        request
            .headers_mut()
            .append(CONTENT_TYPE, HeaderValue::from_static("application/json"));
        request
            .headers_mut()
            .append(CONTENT_TYPE, HeaderValue::from_static("application/json"));
        let error = RequestCodec::decode(&codec, request).await.unwrap_err();
        assert!(matches!(error, FusenError::UnsupportedProtocol(_)));
    }

    #[tokio::test]
    async fn invalid_request_content_type_is_unsupported() {
        let codec = FusenHttpCodec::default();
        let mut request = Request::builder()
            .method("POST")
            .uri("/demo")
            .body(body(b"{}"))
            .unwrap();
        request.headers_mut().insert(
            CONTENT_TYPE,
            HeaderValue::from_bytes(b"\xff").expect("opaque header value"),
        );
        let error = RequestCodec::decode(&codec, request).await.unwrap_err();
        assert!(matches!(error, FusenError::UnsupportedProtocol(_)));
    }

    #[tokio::test]
    async fn invalid_response_content_type_is_invalid_response() {
        let codec = FusenHttpCodec::default();
        let mut response = Response::builder().status(200).body(body(b"{}")).unwrap();
        response
            .headers_mut()
            .append(CONTENT_TYPE, HeaderValue::from_static("application/json"));
        response.headers_mut().append(
            CONTENT_TYPE,
            HeaderValue::from_static("application/problem+json"),
        );
        let error = ResponseCodec::decode(&codec, response).await.unwrap_err();
        assert!(matches!(error, FusenError::InvalidResponse(_)));
    }

    #[tokio::test]
    async fn rejects_malformed_and_non_json_response_content_types() {
        let codec = FusenHttpCodec::default();
        for content_type in ["not a mime", "text/plain"] {
            let response = Response::builder()
                .status(200)
                .header(CONTENT_TYPE, content_type)
                .body(body(b"{}"))
                .unwrap();
            let error = ResponseCodec::decode(&codec, response).await.unwrap_err();
            assert!(matches!(error, FusenError::InvalidResponse(_)));
        }
    }

    #[tokio::test]
    async fn preserves_repeated_headers() {
        let codec = FusenHttpCodec::default();
        let mut request = Request::builder()
            .method("GET")
            .uri("/demo")
            .body(body(b""))
            .unwrap();
        request
            .headers_mut()
            .append(SET_COOKIE, HeaderValue::from_static("a=1"));
        request
            .headers_mut()
            .append(SET_COOKIE, HeaderValue::from_static("b=2"));
        let decoded = RequestCodec::decode(&codec, request).await.unwrap();
        assert_eq!(decoded.headers.get_all(SET_COOKIE).iter().count(), 2);
    }

    #[test]
    fn builds_path_and_query_without_losing_either() {
        let mut request = FusenRequest {
            protocol: WireProtocol::Fusen,
            path: Path {
                method: Method::GET,
                path: "/users/{id}".into(),
            },
            endpoint: Some("http://[::1]:8080/api".parse().unwrap()),
            path_parameters: HashMap::from([("id".into(), "a/b c".into())]),
            query_parameters: QueryParameters::from([("filter".into(), vec!["x y".into()])]),
            headers: Default::default(),
            body: None,
        };
        let uri = build_uri(&mut request).unwrap();
        assert_eq!(
            uri.to_string(),
            "http://[::1]:8080/api/users/a%2Fb%20c?filter=x+y"
        );
    }

    #[test]
    fn preserves_encoded_base_path_without_double_encoding() {
        let mut request = FusenRequest {
            protocol: WireProtocol::Fusen,
            path: Path {
                method: Method::GET,
                path: "/users".into(),
            },
            endpoint: Some("http://localhost/api%20v1".parse().unwrap()),
            path_parameters: HashMap::new(),
            query_parameters: QueryParameters::new(),
            headers: Default::default(),
            body: None,
        };
        assert_eq!(
            build_uri(&mut request).unwrap().to_string(),
            "http://localhost/api%20v1/users"
        );
    }
}
