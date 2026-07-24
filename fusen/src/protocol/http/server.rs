use crate::server::router::HttpRouter;
use hyper_util::{
    rt::{TokioExecutor, TokioIo, TokioTimer},
    server::conn::auto::Builder,
};
use std::{future::Future, io, sync::Arc, time::Duration};
use tokio::{
    net::TcpListener,
    sync::{Semaphore, broadcast},
    task::JoinSet,
    time::Instant,
};

#[derive(Clone, Debug)]
pub(crate) struct TcpServerConfig {
    pub max_connections: usize,
    pub http1_header_read_timeout: Duration,
    pub http2_max_concurrent_streams: u32,
    pub http2_keep_alive_interval: Duration,
    pub http2_keep_alive_timeout: Duration,
}

pub(crate) struct TcpServer;

impl TcpServer {
    pub(crate) async fn run<S, H, F>(
        listener: TcpListener,
        router: HttpRouter,
        shutdown: S,
        on_shutdown: H,
        graceful_timeout: Duration,
        config: TcpServerConfig,
    ) -> io::Result<()>
    where
        S: Future<Output = ()> + Send,
        H: FnOnce(Instant) -> F,
        F: Future<Output = ()> + Send,
    {
        let mut builder = Builder::new(TokioExecutor::new());
        builder
            .http1()
            .keep_alive(true)
            .header_read_timeout(Some(config.http1_header_read_timeout))
            .timer(TokioTimer::new());
        builder
            .http2()
            .max_concurrent_streams(config.http2_max_concurrent_streams)
            .keep_alive_interval(Some(config.http2_keep_alive_interval))
            .keep_alive_timeout(config.http2_keep_alive_timeout)
            .timer(TokioTimer::new());
        let builder = Arc::new(builder);
        let router = Arc::new(router);
        let connection_limit = Arc::new(Semaphore::new(config.max_connections));
        let (connection_shutdown, _) = broadcast::channel::<()>(1);
        let mut connections = JoinSet::new();
        tokio::pin!(shutdown);
        let accept_error = loop {
            tokio::select! {
                accepted = accept_with_permit(&listener, connection_limit.clone()) => match accepted {
                    Ok((stream, permit)) => {
                        let mut shutdown = connection_shutdown.subscribe();
                        let router = router.clone();
                        let builder = builder.clone();
                        connections.spawn(async move {
                            let _permit = permit;
                            let connection = builder.serve_connection_with_upgrades(TokioIo::new(stream), router);
                            tokio::pin!(connection);
                            tokio::select! {
                                result = &mut connection => {
                                    if let Err(error) = result { tracing::debug!(?error, "HTTP connection closed with error"); }
                                }
                                _ = shutdown.recv() => {
                                    connection.as_mut().graceful_shutdown();
                                    if let Err(error) = connection.await { tracing::debug!(?error, "HTTP graceful shutdown failed"); }
                                }
                            }
                        });
                    }
                    Err(error) => break Some(error),
                },
                _ = &mut shutdown => break None,
                result = connections.join_next(), if !connections.is_empty() => {
                    if let Some(Err(error)) = result { tracing::error!(?error, "HTTP connection task panicked"); }
                }
            }
        };

        let deadline = Instant::now() + graceful_timeout;
        if tokio::time::timeout_at(deadline, on_shutdown(deadline))
            .await
            .is_err()
        {
            tracing::error!("service deregistration exceeded the graceful shutdown deadline");
        }
        if connection_shutdown.send(()).is_err() {
            tracing::debug!("no active HTTP connections to drain");
        }
        drop(connection_shutdown);
        if tokio::time::timeout_at(deadline, async {
            while let Some(result) = connections.join_next().await {
                if let Err(error) = result {
                    tracing::error!(?error, "HTTP connection task panicked");
                }
            }
        })
        .await
        .is_err()
        {
            connections.abort_all();
            while connections.join_next().await.is_some() {}
        }
        accept_error.map_or(Ok(()), Err)
    }
}

async fn accept_with_permit(
    listener: &TcpListener,
    limit: Arc<Semaphore>,
) -> io::Result<(tokio::net::TcpStream, tokio::sync::OwnedSemaphorePermit)> {
    let permit = limit
        .acquire_owned()
        .await
        .map_err(|_| io::Error::other("connection semaphore closed"))?;
    let (stream, _) = listener.accept().await?;
    Ok((stream, permit))
}
