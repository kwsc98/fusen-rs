use crate::error::FusenError;
use http::{Request, Response, Version};
use http_body_util::combinators::BoxBody;
use hyper::body::Incoming;
use hyper_tls::HttpsConnector;
use hyper_util::client::legacy::{Client, connect::HttpConnector};
use hyper_util::rt::{TokioExecutor, TokioTimer};
use std::{convert::Infallible, time::Duration};

pub(crate) type HttpSocket =
    Client<HttpsConnector<HttpConnector>, BoxBody<bytes::Bytes, Infallible>>;

/// HTTP/1.1 connection-pool settings shared by every endpoint in a runtime.
#[derive(Clone, Debug)]
pub struct Http1PoolConfig {
    /// Maximum idle connections retained for one endpoint.
    ///
    /// This does not cap concurrent in-use connections. Set it to zero to
    /// disable HTTP/1.1 connection reuse.
    pub max_idle_per_host: usize,
    /// Time after which an idle connection is evicted. `None` disables eviction.
    pub idle_timeout: Option<Duration>,
}

impl Default for Http1PoolConfig {
    fn default() -> Self {
        Self {
            max_idle_per_host: 128,
            idle_timeout: Some(Duration::from_secs(90)),
        }
    }
}

/// HTTP/2 connection-pool settings shared by every endpoint in a runtime.
#[derive(Clone, Debug)]
pub struct Http2PoolConfig {
    /// Independent multiplexed connections maintained for one endpoint.
    ///
    /// Connections are opened lazily and selected by endpoint and request ID.
    pub connections_per_host: usize,
    /// Time after which an idle connection is evicted. `None` disables eviction.
    pub idle_timeout: Option<Duration>,
    /// Interval between HTTP/2 keep-alive pings. `None` disables pings.
    pub keep_alive_interval: Option<Duration>,
    /// Maximum wait for an HTTP/2 keep-alive acknowledgement.
    pub keep_alive_timeout: Duration,
    /// Whether to send keep-alive pings when no streams are active.
    pub keep_alive_while_idle: bool,
}

impl Default for Http2PoolConfig {
    fn default() -> Self {
        Self {
            connections_per_host: 1,
            idle_timeout: Some(Duration::from_secs(90)),
            keep_alive_interval: None,
            keep_alive_timeout: Duration::from_secs(20),
            keep_alive_while_idle: false,
        }
    }
}

#[derive(Debug)]
pub(crate) struct HttpClient {
    http1_client: HttpSocket,
    http2_clients: Box<[HttpSocket]>,
}

impl HttpClient {
    pub(crate) fn new(
        connect_timeout: Duration,
        http1_pool: &Http1PoolConfig,
        http2_pool: &Http2PoolConfig,
    ) -> Self {
        let mut connector = HttpConnector::new();
        connector.set_connect_timeout(Some(connect_timeout));
        connector.set_keepalive(Some(Duration::from_secs(90)));
        connector.enforce_http(false);
        let mut http1_builder = Client::builder(TokioExecutor::new());
        http1_builder
            .pool_timer(TokioTimer::new())
            .pool_idle_timeout(http1_pool.idle_timeout)
            .pool_max_idle_per_host(http1_pool.max_idle_per_host);
        let http1_client =
            http1_builder.build(HttpsConnector::new_with_connector(connector.clone()));

        debug_assert!(http2_pool.connections_per_host > 0);
        let http2_clients = (0..http2_pool.connections_per_host)
            .map(|_| {
                let mut builder = Client::builder(TokioExecutor::new());
                builder
                    .pool_timer(TokioTimer::new())
                    .pool_idle_timeout(http2_pool.idle_timeout)
                    .pool_max_idle_per_host(1)
                    .http2_only(true)
                    .http2_keep_alive_interval(http2_pool.keep_alive_interval)
                    .http2_keep_alive_timeout(http2_pool.keep_alive_timeout)
                    .http2_keep_alive_while_idle(http2_pool.keep_alive_while_idle);
                builder.build(HttpsConnector::new_with_connector(connector.clone()))
            })
            .collect();
        Self {
            http1_client,
            http2_clients,
        }
    }

    pub(crate) async fn send_http_request(
        &self,
        request: Request<BoxBody<bytes::Bytes, Infallible>>,
    ) -> Result<Response<Incoming>, FusenError> {
        let client = match request.version() {
            Version::HTTP_2 => {
                let authority = request
                    .uri()
                    .authority()
                    .map(|authority| authority.as_str().as_bytes())
                    .unwrap_or_default();
                let request_id = request
                    .headers()
                    .get("x-request-id")
                    .map(|value| value.as_bytes())
                    .unwrap_or_default();
                let index = stable_shard_index(authority, request_id, self.http2_clients.len());
                &self.http2_clients[index]
            }
            _ => &self.http1_client,
        };
        client
            .request(request)
            .await
            .map_err(|error| FusenError::internal("HTTP request failed", error))
    }
}

fn stable_shard_index(authority: &[u8], request_id: &[u8], shard_count: usize) -> usize {
    const FNV_OFFSET_BASIS: u64 = 0xcbf2_9ce4_8422_2325;
    const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;

    let mut hash = FNV_OFFSET_BASIS;
    for part in [authority, request_id] {
        for byte in (part.len() as u64).to_le_bytes().iter().chain(part) {
            hash ^= u64::from(*byte);
            hash = hash.wrapping_mul(FNV_PRIME);
        }
    }
    (hash % shard_count as u64) as usize
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn creates_the_configured_number_of_http2_connection_shards() {
        let http2 = Http2PoolConfig {
            connections_per_host: 4,
            ..Http2PoolConfig::default()
        };
        let client = HttpClient::new(Duration::from_secs(1), &Http1PoolConfig::default(), &http2);

        assert_eq!(client.http2_clients.len(), 4);
    }

    #[test]
    fn http2_shard_mapping_is_stable_and_covers_all_shards() {
        let authority = b"127.0.0.1:8080";
        assert_eq!(
            stable_shard_index(authority, b"request-42", 4),
            stable_shard_index(authority, b"request-42", 4)
        );

        let mut selected = [false; 4];
        for request_id in 0..256 {
            let request_id = format!("request-{request_id}");
            selected[stable_shard_index(authority, request_id.as_bytes(), 4)] = true;
        }
        assert!(selected.into_iter().all(|value| value));
    }

    #[test]
    fn endpoint_mappings_do_not_depend_on_other_endpoints() {
        let first = stable_shard_index(b"one.example:443", b"request-7", 8);
        let _other = stable_shard_index(b"two.example:443", b"request-99", 8);
        let repeated = stable_shard_index(b"one.example:443", b"request-7", 8);

        assert_eq!(first, repeated);
    }
}
