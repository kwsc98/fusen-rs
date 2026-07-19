use crate::server::router::Router;
use hyper_util::{
    rt::{TokioExecutor, TokioIo},
    server::conn::auto::Builder,
};
use std::{future::Future, io, sync::Arc, time::Duration};
use tokio::{net::TcpListener, sync::broadcast, task::JoinSet};

pub struct TcpServer;

impl TcpServer {
    pub async fn run<S, H>(
        listener: TcpListener,
        router: Router,
        shutdown: S,
        on_shutdown: H,
        graceful_timeout: Duration,
    ) -> io::Result<()>
    where
        S: Future<Output = ()> + Send,
        H: Future<Output = ()> + Send,
    {
        let mut builder = Builder::new(TokioExecutor::new());
        builder.http2().max_concurrent_streams(None);
        builder.http1().keep_alive(true);
        let builder = Arc::new(builder);
        let router = Arc::new(router);
        let (connection_shutdown, _) = broadcast::channel::<()>(1);
        let mut connections = JoinSet::new();
        tokio::pin!(shutdown);
        let accept_error = loop {
            tokio::select! {
                result = listener.accept() => match result {
                    Ok((stream, _)) => {
                        let mut shutdown = connection_shutdown.subscribe();
                        let router = router.clone();
                        let builder = builder.clone();
                        connections.spawn(async move {
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

        on_shutdown.await;
        if connection_shutdown.send(()).is_err() {
            tracing::debug!("no active HTTP connections to drain");
        }
        drop(connection_shutdown);
        if tokio::time::timeout(graceful_timeout, async {
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
