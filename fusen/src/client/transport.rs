use super::config::ClientHttpConfig;
use crate::{RpcCategory, RpcError, wire::GuardedBody};
use http::{Request, Response, Version};
use hyper::body::Incoming;
use hyper_util::{
    client::legacy::{Client, connect::HttpConnector},
    rt::{TokioExecutor, TokioTimer},
};
use std::time::Duration;

type Socket = Client<HttpConnector, GuardedBody>;

#[derive(Clone)]
pub(crate) struct HttpTransport {
    http1: Socket,
    http2: Box<[Socket]>,
}

impl HttpTransport {
    pub(crate) fn new(connect_timeout: Duration, config: &ClientHttpConfig) -> Self {
        let mut connector = HttpConnector::new();
        connector.set_connect_timeout(Some(connect_timeout));
        connector.set_keepalive(Some(Duration::from_secs(90)));
        connector.enforce_http(true);

        let mut http1 = Client::builder(TokioExecutor::new());
        http1
            .pool_timer(TokioTimer::new())
            .pool_idle_timeout(config.pool_idle_timeout())
            .pool_max_idle_per_host(config.http1_max_idle_per_host())
            .http1_writev(true);
        let http1 = http1.build(connector.clone());

        let http2 = (0..config.http2_connections_per_host())
            .map(|_| {
                let mut builder = Client::builder(TokioExecutor::new());
                builder
                    .pool_timer(TokioTimer::new())
                    .pool_idle_timeout(config.pool_idle_timeout())
                    .pool_max_idle_per_host(1)
                    .http2_only(true)
                    .http2_keep_alive_interval(config.http2_keep_alive_interval())
                    .http2_keep_alive_timeout(config.http2_keep_alive_timeout())
                    .http2_keep_alive_while_idle(false);
                builder.build(connector.clone())
            })
            .collect();
        Self { http1, http2 }
    }

    pub(crate) async fn send(
        &self,
        request: Request<GuardedBody>,
    ) -> Result<Response<Incoming>, TransportFailure> {
        let client = if request.version() == Version::HTTP_2 {
            let authority = request
                .uri()
                .authority()
                .map(|value| value.as_str().as_bytes())
                .unwrap_or_default();
            let request_id = request
                .headers()
                .get("x-request-id")
                .map(http::HeaderValue::as_bytes)
                .unwrap_or_default();
            &self.http2[stable_shard(authority, request_id, self.http2.len())]
        } else {
            &self.http1
        };
        client.request(request).await.map_err(|error| {
            let kind = if error.is_connect() {
                TransportFailureKind::Connect
            } else {
                TransportFailureKind::Io
            };
            TransportFailure { kind, error }
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TransportFailureKind {
    Connect,
    Io,
}

#[derive(Debug)]
pub(crate) struct TransportFailure {
    pub kind: TransportFailureKind,
    error: hyper_util::client::legacy::Error,
}

impl TransportFailure {
    pub(crate) fn into_rpc(self) -> RpcError {
        RpcError::internal("plaintext HTTP transport failed", self.error).mark_retryable()
    }
}

fn stable_shard(authority: &[u8], request_id: &[u8], count: usize) -> usize {
    const OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
    const PRIME: u64 = 0x0000_0100_0000_01b3;
    let mut hash = OFFSET;
    for part in [authority, request_id] {
        for byte in (part.len() as u64).to_le_bytes().iter().chain(part) {
            hash ^= u64::from(*byte);
            hash = hash.wrapping_mul(PRIME);
        }
    }
    (hash % count as u64) as usize
}

pub(crate) fn circuit_open() -> RpcError {
    RpcError::framework(
        RpcCategory::Unavailable,
        "circuit_open",
        "circuit breaker rejected the attempt",
    )
}
