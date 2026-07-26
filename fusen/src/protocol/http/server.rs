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

pub(crate) enum ShutdownCompletion<E> {
    Completed(Result<(), E>),
    DeadlineExceeded { cleanup: Option<Result<(), E>> },
}

pub(crate) struct TcpServerOutcome<E> {
    pub accept_error: Option<io::Error>,
    pub shutdown: ShutdownCompletion<E>,
}

impl TcpServer {
    pub(crate) async fn run<S, H, F, E>(
        listener: TcpListener,
        router: HttpRouter,
        shutdown: S,
        on_shutdown: H,
        graceful_timeout: Duration,
        config: TcpServerConfig,
    ) -> TcpServerOutcome<E>
    where
        S: Future<Output = ()> + Send,
        H: FnOnce(Instant) -> F,
        F: Future<Output = Result<(), E>> + Send,
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
                biased;
                _ = &mut shutdown => break None,
                result = connections.join_next(), if !connections.is_empty() => {
                    if let Some(Err(error)) = result { tracing::error!(?error, "HTTP connection task panicked"); }
                },
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
                }
            }
        };

        let deadline = Instant::now() + graceful_timeout;
        drop(listener);
        if connection_shutdown.send(()).is_err() {
            tracing::debug!("no active HTTP connections to drain");
        }
        drop(connection_shutdown);
        let cleanup = on_shutdown(deadline);
        let shutdown = coordinate_shutdown(&mut connections, cleanup, deadline).await;
        drop(connections);
        TcpServerOutcome {
            accept_error,
            shutdown,
        }
    }
}

async fn coordinate_shutdown<F, E>(
    connections: &mut JoinSet<()>,
    cleanup: F,
    deadline: Instant,
) -> ShutdownCompletion<E>
where
    F: Future<Output = Result<(), E>>,
{
    tokio::pin!(cleanup);
    let mut cleanup_result = None;
    let mut connections_drained = false;
    let shutdown_completed = {
        let drain = drain_connections(connections);
        tokio::pin!(drain);
        tokio::time::timeout_at(deadline, async {
            loop {
                // timeout_at polls its inner future first, so guard the exact boundary.
                if Instant::now() >= deadline {
                    std::future::pending::<()>().await;
                }
                tokio::select! {
                    result = &mut cleanup, if cleanup_result.is_none() => {
                        cleanup_result = Some(result);
                        if connections_drained {
                            break;
                        }
                    }
                    () = &mut drain, if !connections_drained => {
                        connections_drained = true;
                        if cleanup_result.is_some() {
                            break;
                        }
                    }
                }
            }
        })
        .await
        .is_ok()
    };
    if !shutdown_completed {
        connections.abort_all();
        ShutdownCompletion::DeadlineExceeded {
            cleanup: cleanup_result,
        }
    } else {
        ShutdownCompletion::Completed(
            cleanup_result.expect("cleanup completed before shutdown finished"),
        )
    }
}

async fn drain_connections(connections: &mut JoinSet<()>) {
    while let Some(result) = connections.join_next().await {
        if let Err(error) = result {
            tracing::error!(?error, "HTTP connection task panicked");
        }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test(start_paused = true)]
    async fn deadline_preserves_a_completed_cleanup_error() {
        let mut connections = JoinSet::new();
        connections.spawn(std::future::pending());

        let shutdown = coordinate_shutdown(
            &mut connections,
            async { Err::<(), _>("cleanup failed") },
            Instant::now() + Duration::from_secs(5),
        )
        .await;

        assert!(matches!(
            shutdown,
            ShutdownCompletion::DeadlineExceeded {
                cleanup: Some(Err("cleanup failed"))
            }
        ));
    }

    #[tokio::test(start_paused = true)]
    async fn completed_cleanup_and_connection_drain_finish_before_deadline() {
        let mut connections = JoinSet::new();
        connections.spawn(async {});

        let shutdown = coordinate_shutdown(
            &mut connections,
            async { Ok::<_, std::convert::Infallible>(()) },
            Instant::now() + Duration::from_secs(5),
        )
        .await;

        assert!(matches!(shutdown, ShutdownCompletion::Completed(Ok(()))));
    }

    #[tokio::test(start_paused = true)]
    async fn deadline_wins_when_shutdown_work_completes_at_the_same_instant() {
        let mut connections = JoinSet::new();
        connections.spawn(async {
            tokio::time::sleep(Duration::from_secs(5)).await;
        });

        let shutdown = coordinate_shutdown(
            &mut connections,
            async {
                tokio::time::sleep(Duration::from_secs(5)).await;
                Ok::<_, std::convert::Infallible>(())
            },
            Instant::now() + Duration::from_secs(5),
        )
        .await;

        assert!(matches!(
            shutdown,
            ShutdownCompletion::DeadlineExceeded { .. }
        ));
    }
}
