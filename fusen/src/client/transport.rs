use super::config::ClientHttpConfig;
use crate::{ClientError, ClientErrorKind, Error, ErrorCategory, RetryHint, wire::GuardedBody};
use http::{Request, Response as HttpResponse, Uri, Version, uri::Scheme};
use hyper::body::Incoming;
use hyper_rustls::{ConfigBuilderExt, HttpsConnector, HttpsConnectorBuilder, MaybeHttpsStream};
use hyper_util::{
    client::legacy::{Client, connect::HttpConnector},
    rt::{TokioExecutor, TokioTimer},
};
use rustls::ClientConfig as TlsClientConfig;
use std::{
    future::Future,
    io,
    pin::Pin,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    task::{Context as TaskContext, Poll},
    time::Duration,
};
use tower_service::Service;

type Connector = HttpsConnector<HttpConnector>;
type Http1Socket = Client<Connector, GuardedBody>;
type AutoSocket = Client<Connector, GuardedBody>;
type Http2Socket = Client<RequireH2Alpn<Connector>, GuardedBody>;
type BoxError = Box<dyn std::error::Error + Send + Sync>;

#[derive(Clone)]
pub(crate) struct HttpTransport {
    http1: Http1Socket,
    auto: Box<[AutoSocket]>,
    http2: Box<[Http2Socket]>,
    next_auto_shard: Arc<AtomicUsize>,
    next_http2_shard: Arc<AtomicUsize>,
}

impl HttpTransport {
    pub(crate) fn new(
        connect_timeout: Duration,
        config: &ClientHttpConfig,
    ) -> Result<Self, ClientError> {
        let provider = Arc::new(rustls::crypto::ring::default_provider());
        let tls_config = TlsClientConfig::builder_with_provider(provider)
            .with_safe_default_protocol_versions()
            .map_err(|error| {
                ClientError::with_source(
                    ClientErrorKind::Build,
                    "failed to initialize the client TLS protocol versions",
                    error,
                )
            })?
            .with_webpki_roots()
            .with_no_client_auth();
        Ok(Self::with_tls_config(connect_timeout, config, tls_config))
    }

    fn with_tls_config(
        connect_timeout: Duration,
        config: &ClientHttpConfig,
        tls_config: TlsClientConfig,
    ) -> Self {
        let http1_connector = HttpsConnectorBuilder::new()
            .with_tls_config(tls_config.clone())
            .https_or_http()
            .enable_http1()
            .wrap_connector(http_connector(connect_timeout));
        let auto_connector = HttpsConnectorBuilder::new()
            .with_tls_config(tls_config.clone())
            .https_or_http()
            .enable_http1()
            .enable_http2()
            .wrap_connector(http_connector(connect_timeout));
        let http2_connector = RequireH2Alpn {
            inner: HttpsConnectorBuilder::new()
                .with_tls_config(tls_config)
                .https_or_http()
                .enable_http2()
                .wrap_connector(http_connector(connect_timeout)),
        };

        let mut http1 = Client::builder(TokioExecutor::new());
        http1
            .pool_timer(TokioTimer::new())
            .pool_idle_timeout(config.pool_idle_timeout())
            .pool_max_idle_per_host(config.http1_max_idle_per_host())
            .http1_writev(true);
        let http1 = http1.build(http1_connector);

        let auto = (0..config.http2_connections_per_host())
            .map(|_| {
                let mut builder = Client::builder(TokioExecutor::new());
                builder
                    .pool_timer(TokioTimer::new())
                    .pool_idle_timeout(config.pool_idle_timeout())
                    .pool_max_idle_per_host(config.http1_max_idle_per_host())
                    .http1_writev(true)
                    .http2_keep_alive_interval(config.http2_keep_alive_interval())
                    .http2_keep_alive_timeout(config.http2_keep_alive_timeout())
                    .http2_keep_alive_while_idle(false);
                builder.build(auto_connector.clone())
            })
            .collect();

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
                builder.build(http2_connector.clone())
            })
            .collect();
        Self {
            http1,
            auto,
            http2,
            next_auto_shard: Arc::new(AtomicUsize::new(0)),
            next_http2_shard: Arc::new(AtomicUsize::new(0)),
        }
    }

    pub(crate) async fn send(
        &self,
        request: Request<GuardedBody>,
        auto_negotiate: bool,
    ) -> Result<HttpResponse<Incoming>, TransportFailure> {
        let response = if auto_negotiate {
            let shard = request_shard(&request, self.auto.len(), &self.next_auto_shard);
            self.auto[shard].request(request).await
        } else if request.version() == Version::HTTP_2 {
            let shard = request_shard(&request, self.http2.len(), &self.next_http2_shard);
            self.http2[shard].request(request).await
        } else {
            self.http1.request(request).await
        };
        response.map_err(|error| {
            let kind = if error.is_connect() {
                TransportFailureKind::Connect
            } else {
                TransportFailureKind::Io
            };
            TransportFailure { kind, error }
        })
    }
}

fn request_shard<B>(request: &Request<B>, count: usize, next: &AtomicUsize) -> usize {
    let authority = request
        .uri()
        .authority()
        .map(|value| value.as_str().as_bytes())
        .unwrap_or_default();
    match request
        .headers()
        .get("x-request-id")
        .map(http::HeaderValue::as_bytes)
        .filter(|request_id| !request_id.is_empty())
    {
        Some(request_id) => stable_shard(authority, request_id, count),
        None => next.fetch_add(1, Ordering::Relaxed) % count,
    }
}

#[derive(Clone)]
struct RequireH2Alpn<C> {
    inner: C,
}

impl<C, T> Service<Uri> for RequireH2Alpn<C>
where
    C: Service<Uri, Response = MaybeHttpsStream<T>, Error = BoxError>,
    C::Future: Send + 'static,
    T: Send + 'static,
{
    type Response = MaybeHttpsStream<T>;
    type Error = BoxError;
    type Future = Pin<Box<dyn Future<Output = Result<Self::Response, Self::Error>> + Send>>;

    fn poll_ready(&mut self, context: &mut TaskContext<'_>) -> Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(context)
    }

    fn call(&mut self, destination: Uri) -> Self::Future {
        let requires_h2_alpn = destination.scheme() == Some(&Scheme::HTTPS);
        let connecting = self.inner.call(destination);
        Box::pin(async move {
            let stream = connecting.await?;
            if requires_h2_alpn {
                let negotiated_h2 = match &stream {
                    MaybeHttpsStream::Https(tls) => {
                        tls.inner().get_ref().1.alpn_protocol() == Some(b"h2")
                    }
                    MaybeHttpsStream::Http(_) => false,
                };
                if !negotiated_h2 {
                    return Err(io::Error::other(
                        "HTTPS connection did not negotiate the required ALPN h2 protocol",
                    )
                    .into());
                }
            }
            Ok(stream)
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
    pub(crate) fn into_error(self) -> Error {
        Error::internal("HTTP transport failed", self.error).with_retry_hint(RetryHint::Retryable)
    }
}

fn http_connector(connect_timeout: Duration) -> HttpConnector {
    let mut connector = HttpConnector::new();
    connector.set_connect_timeout(Some(connect_timeout));
    connector.set_keepalive(Some(Duration::from_secs(90)));
    connector.enforce_http(false);
    connector
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

pub(crate) fn circuit_open() -> Error {
    Error::framework(
        ErrorCategory::Unavailable,
        "circuit_open",
        "circuit breaker rejected the attempt",
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use bytes::Bytes;
    use http_body_util::{BodyExt, Full};
    use hyper::service::service_fn;
    use hyper_util::rt::TokioIo;
    use rcgen::{CertifiedKey, generate_simple_self_signed};
    use rustls::{
        RootCertStore, ServerConfig as TlsServerConfig, SupportedProtocolVersion,
        pki_types::{CertificateDer, PrivatePkcs8KeyDer},
    };
    use std::{convert::Infallible, sync::Arc};
    use tokio::{net::TcpListener, task::JoinHandle};
    use tokio_rustls::TlsAcceptor;

    struct TlsFixture {
        endpoint: String,
        certificate: CertificateDer<'static>,
        task: JoinHandle<()>,
    }

    async fn spawn_tls_server(
        version: Version,
        certificate_name: &str,
        advertise_alpn: bool,
        tls_version: &'static SupportedProtocolVersion,
    ) -> TlsFixture {
        let CertifiedKey { cert, key_pair } =
            generate_simple_self_signed(vec![certificate_name.to_owned()]).unwrap();
        let certificate = cert.der().clone();
        let private_key = PrivatePkcs8KeyDer::from(key_pair.serialize_der()).into();
        let provider = Arc::new(rustls::crypto::ring::default_provider());
        let mut server_config = TlsServerConfig::builder_with_provider(provider)
            .with_protocol_versions(&[tls_version])
            .unwrap()
            .with_no_client_auth()
            .with_single_cert(vec![certificate.clone()], private_key)
            .unwrap();
        server_config.alpn_protocols = if !advertise_alpn {
            Vec::new()
        } else if version == Version::HTTP_2 {
            vec![b"h2".to_vec()]
        } else {
            vec![b"http/1.1".to_vec()]
        };

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let acceptor = TlsAcceptor::from(Arc::new(server_config));
        let task = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let Ok(stream) = acceptor.accept(stream).await else {
                return;
            };
            let service = service_fn(|_| async {
                Ok::<_, Infallible>(HttpResponse::new(Full::new(Bytes::from_static(b"secure"))))
            });
            if version == Version::HTTP_2 {
                let _ = hyper::server::conn::http2::Builder::new(TokioExecutor::new())
                    .serve_connection(TokioIo::new(stream), service)
                    .await;
            } else {
                let _ = hyper::server::conn::http1::Builder::new()
                    .serve_connection(TokioIo::new(stream), service)
                    .await;
            }
        });
        TlsFixture {
            endpoint: format!("https://127.0.0.1:{port}/rpc"),
            certificate,
            task,
        }
    }

    fn client_tls_config(certificate: Option<CertificateDer<'static>>) -> TlsClientConfig {
        let mut roots = RootCertStore::empty();
        if let Some(certificate) = certificate {
            roots.add(certificate).unwrap();
        }
        let provider = Arc::new(rustls::crypto::ring::default_provider());
        TlsClientConfig::builder_with_provider(provider)
            .with_safe_default_protocol_versions()
            .unwrap()
            .with_root_certificates(roots)
            .with_no_client_auth()
    }

    async fn send_tls_request(
        version: Version,
        fixture: &TlsFixture,
        tls_config: TlsClientConfig,
    ) -> Result<Bytes, TransportFailure> {
        let transport = HttpTransport::with_tls_config(
            Duration::from_secs(1),
            &ClientHttpConfig::default(),
            tls_config,
        );
        let request = Request::builder()
            .method("POST")
            .version(version)
            .uri(&fixture.endpoint)
            .header("x-request-id", "tls-test")
            .body(GuardedBody::new(Bytes::new(), None))
            .unwrap();
        let response = transport.send(request, false).await?;
        Ok(response.into_body().collect().await.unwrap().to_bytes())
    }

    async fn send_auto_tls_request(
        fixture: &TlsFixture,
        tls_config: TlsClientConfig,
    ) -> Result<(Version, Bytes), TransportFailure> {
        let transport = HttpTransport::with_tls_config(
            Duration::from_secs(1),
            &ClientHttpConfig::default(),
            tls_config,
        );
        let request = Request::builder()
            .method("POST")
            .uri(&fixture.endpoint)
            .body(GuardedBody::new(Bytes::new(), None))
            .unwrap();
        let response = transport.send(request, true).await?;
        let version = response.version();
        let body = response.into_body().collect().await.unwrap().to_bytes();
        Ok((version, body))
    }

    #[tokio::test]
    async fn configured_http2_shards_are_built_for_explicit_and_auto_pools() {
        let config = ClientHttpConfig::builder()
            .http2_connections_per_host(3)
            .build()
            .unwrap();
        let transport = HttpTransport::with_tls_config(
            Duration::from_secs(1),
            &config,
            client_tls_config(None),
        );

        assert_eq!(transport.auto.len(), 3);
        assert_eq!(transport.http2.len(), 3);
    }

    #[test]
    fn requests_without_control_ids_round_robin_across_shards() {
        let request = Request::builder()
            .uri("https://service.example/items")
            .body(())
            .unwrap();
        let next = AtomicUsize::new(0);

        assert_eq!(request_shard(&request, 3, &next), 0);
        assert_eq!(request_shard(&request, 3, &next), 1);
        assert_eq!(request_shard(&request, 3, &next), 2);
        assert_eq!(request_shard(&request, 3, &next), 0);
    }

    #[test]
    fn requests_with_control_ids_keep_stable_shard_selection() {
        let request = Request::builder()
            .uri("https://service.example/items")
            .header("x-request-id", "request-1")
            .body(())
            .unwrap();
        let next = AtomicUsize::new(0);

        let selected = request_shard(&request, 3, &next);
        assert_eq!(request_shard(&request, 3, &next), selected);
        assert_eq!(next.load(Ordering::Relaxed), 0);
    }

    #[tokio::test]
    async fn https_supports_tls12_and_tls13_over_http1_and_http2() {
        for tls_version in [&rustls::version::TLS12, &rustls::version::TLS13] {
            for version in [Version::HTTP_11, Version::HTTP_2] {
                let fixture = spawn_tls_server(version, "127.0.0.1", true, tls_version).await;
                let body = send_tls_request(
                    version,
                    &fixture,
                    client_tls_config(Some(fixture.certificate.clone())),
                )
                .await
                .unwrap();
                assert_eq!(body, "secure");
                fixture.task.abort();
            }
        }
    }

    #[tokio::test]
    async fn https_auto_negotiates_http2_with_alpn() {
        let fixture =
            spawn_tls_server(Version::HTTP_2, "127.0.0.1", true, &rustls::version::TLS13).await;
        let (version, body) = send_auto_tls_request(
            &fixture,
            client_tls_config(Some(fixture.certificate.clone())),
        )
        .await
        .unwrap();
        assert_eq!(version, Version::HTTP_2);
        assert_eq!(body, "secure");
        fixture.task.abort();
    }

    #[tokio::test]
    async fn https_auto_falls_back_to_http1_with_alpn() {
        let fixture =
            spawn_tls_server(Version::HTTP_11, "127.0.0.1", true, &rustls::version::TLS13).await;
        let (version, body) = send_auto_tls_request(
            &fixture,
            client_tls_config(Some(fixture.certificate.clone())),
        )
        .await
        .unwrap();
        assert_eq!(version, Version::HTTP_11);
        assert_eq!(body, "secure");
        fixture.task.abort();
    }

    #[tokio::test]
    async fn https_rejects_untrusted_certificates_without_plaintext_fallback() {
        let fixture =
            spawn_tls_server(Version::HTTP_11, "127.0.0.1", true, &rustls::version::TLS13).await;
        let error = send_tls_request(Version::HTTP_11, &fixture, client_tls_config(None))
            .await
            .unwrap_err();
        assert_eq!(error.kind, TransportFailureKind::Connect);
        fixture.task.abort();
    }

    #[tokio::test]
    async fn https_rejects_hostname_mismatches() {
        let fixture =
            spawn_tls_server(Version::HTTP_11, "localhost", true, &rustls::version::TLS13).await;
        let error = send_tls_request(
            Version::HTTP_11,
            &fixture,
            client_tls_config(Some(fixture.certificate.clone())),
        )
        .await
        .unwrap_err();
        assert_eq!(error.kind, TransportFailureKind::Connect);
        fixture.task.abort();
    }

    #[tokio::test]
    async fn https_http2_requires_the_server_to_negotiate_h2_alpn() {
        let fixture =
            spawn_tls_server(Version::HTTP_2, "127.0.0.1", false, &rustls::version::TLS13).await;
        let error = send_tls_request(
            Version::HTTP_2,
            &fixture,
            client_tls_config(Some(fixture.certificate.clone())),
        )
        .await
        .unwrap_err();
        assert_eq!(error.kind, TransportFailureKind::Connect);
        fixture.task.abort();
    }
}
