use crate::{Arguments, Body, Error, Response};
use bytes::Bytes;
use fusen_contract::{MethodDescriptor, ServiceDescriptor};
use http::{HeaderMap, Method, StatusCode, Version};

/// Immutable service invocation data presented to a request encoder.
pub struct RequestEncoding<'a> {
    service: &'static ServiceDescriptor,
    method: &'static MethodDescriptor,
    arguments: &'a Arguments,
    headers: &'a HeaderMap,
}

impl<'a> RequestEncoding<'a> {
    pub(crate) const fn new(
        service: &'static ServiceDescriptor,
        method: &'static MethodDescriptor,
        arguments: &'a Arguments,
        headers: &'a HeaderMap,
    ) -> Self {
        Self {
            service,
            method,
            arguments,
            headers,
        }
    }

    /// Returns the complete service contract.
    pub const fn service(&self) -> &'static ServiceDescriptor {
        self.service
    }

    /// Returns the selected method contract.
    pub const fn method(&self) -> &'static MethodDescriptor {
        self.method
    }

    /// Returns serialized invocation arguments.
    pub const fn arguments(&self) -> &Arguments {
        self.arguments
    }

    /// Returns application-provided request headers.
    pub const fn headers(&self) -> &HeaderMap {
        self.headers
    }
}

/// HTTP semantic request produced by a [`RequestEncoder`].
#[derive(Clone)]
pub struct EncodedRequest {
    method: Method,
    path_and_query: String,
    headers: HeaderMap,
    body: Bytes,
}

impl std::fmt::Debug for EncodedRequest {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("EncodedRequest")
            .field("method", &self.method)
            .field("path_and_query", &"<omitted>")
            .field("header_count", &self.headers.len())
            .field("body_length", &self.body.len())
            .finish()
    }
}

impl EncodedRequest {
    /// Creates HTTP semantic request parts. Runtime validation is applied before network I/O.
    pub fn new(
        method: Method,
        path_and_query: impl Into<String>,
        headers: HeaderMap,
        body: Bytes,
    ) -> Self {
        Self {
            method,
            path_and_query: path_and_query.into(),
            headers,
            body,
        }
    }

    /// Returns the HTTP method.
    pub const fn method(&self) -> &Method {
        &self.method
    }

    /// Returns the relative path and optional query string.
    pub fn path_and_query(&self) -> &str {
        &self.path_and_query
    }

    /// Returns encoded request headers.
    pub const fn headers(&self) -> &HeaderMap {
        &self.headers
    }

    /// Returns encoded request bytes.
    pub const fn body(&self) -> &Bytes {
        &self.body
    }

    pub(crate) fn into_parts(self) -> (Method, String, HeaderMap, Bytes) {
        (self.method, self.path_and_query, self.headers, self.body)
    }
}

/// Bounded HTTP response data presented to response and error decoders.
#[derive(Clone)]
pub struct BufferedResponse {
    status: StatusCode,
    version: Version,
    headers: HeaderMap,
    body: Bytes,
    request_id: String,
    invocation_controls: bool,
}

impl std::fmt::Debug for BufferedResponse {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("BufferedResponse")
            .field("status", &self.status)
            .field("version", &self.version)
            .field("header_count", &self.headers.len())
            .field("body_length", &self.body.len())
            .finish()
    }
}

impl BufferedResponse {
    pub(crate) fn new(
        status: StatusCode,
        version: Version,
        headers: HeaderMap,
        body: Bytes,
        request_id: String,
        invocation_controls: bool,
    ) -> Self {
        Self {
            status,
            version,
            headers,
            body,
            request_id,
            invocation_controls,
        }
    }

    /// Returns the HTTP status.
    pub const fn status(&self) -> StatusCode {
        self.status
    }

    /// Returns the negotiated HTTP version.
    pub const fn version(&self) -> Version {
        self.version
    }

    /// Returns response headers after hop-by-hop and runtime control headers were removed.
    pub const fn headers(&self) -> &HeaderMap {
        &self.headers
    }

    /// Returns bounded response bytes.
    pub const fn body(&self) -> &Bytes {
        &self.body
    }

    pub(crate) fn request_id(&self) -> &str {
        &self.request_id
    }

    pub(crate) const fn invocation_controls(&self) -> bool {
        self.invocation_controls
    }

    /// Consumes the response into HTTP semantic parts.
    pub fn into_parts(self) -> (StatusCode, Version, HeaderMap, Bytes) {
        (self.status, self.version, self.headers, self.body)
    }
}

/// Encodes one invocation into HTTP semantic request parts.
pub trait RequestEncoder: Send + Sync + 'static {
    /// Encodes the request without performing transport I/O.
    fn encode(&self, request: RequestEncoding<'_>) -> Result<EncodedRequest, Error>;
}

/// Decodes one successful, bounded HTTP response.
pub trait ResponseDecoder: Send + Sync + 'static {
    /// Decodes a 2xx response into the runtime response body representation.
    ///
    /// A returned error is attributed to the remote response. A panic is isolated separately as
    /// a local codec failure.
    fn decode(
        &self,
        method: &'static MethodDescriptor,
        response: BufferedResponse,
    ) -> Result<Response<Body>, Error>;
}

/// Normalizes one non-success, bounded HTTP response.
pub trait ErrorDecoder: Send + Sync + 'static {
    /// Decodes a non-2xx response without exposing its raw body automatically.
    ///
    /// The runtime attributes the returned error to the remote endpoint, attaches the trusted
    /// invocation request ID, and replaces framework-owned response headers.
    fn decode(&self, method: &'static MethodDescriptor, response: BufferedResponse) -> Error;
}

impl<T> RequestEncoder for std::sync::Arc<T>
where
    T: RequestEncoder + ?Sized,
{
    fn encode(&self, request: RequestEncoding<'_>) -> Result<EncodedRequest, Error> {
        (**self).encode(request)
    }
}

impl<T> ResponseDecoder for std::sync::Arc<T>
where
    T: ResponseDecoder + ?Sized,
{
    fn decode(
        &self,
        method: &'static MethodDescriptor,
        response: BufferedResponse,
    ) -> Result<Response<Body>, Error> {
        (**self).decode(method, response)
    }
}

impl<T> ErrorDecoder for std::sync::Arc<T>
where
    T: ErrorDecoder + ?Sized,
{
    fn decode(&self, method: &'static MethodDescriptor, response: BufferedResponse) -> Error {
        (**self).decode(method, response)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use http::header::AUTHORIZATION;

    #[test]
    fn encoded_request_debug_never_expands_values() {
        let mut headers = HeaderMap::new();
        headers.insert(
            AUTHORIZATION,
            http::HeaderValue::from_static("private-authorization"),
        );
        let request = EncodedRequest::new(
            Method::POST,
            "/users/private-path?token=private-query",
            headers,
            Bytes::from_static(b"private-body"),
        );

        let debug = format!("{request:?}");
        assert!(debug.contains("method: POST"));
        assert!(debug.contains("header_count: 1"));
        assert!(debug.contains("body_length: 12"));
        assert!(!debug.contains("private-"));
    }
}
