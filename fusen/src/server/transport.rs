use super::http::HttpApp;
use hyper::rt::Executor;
#[cfg(test)]
use hyper_util::rt::TokioExecutor;
use hyper_util::{
    rt::{TokioIo, TokioTimer},
    server::conn::auto::Builder,
};
use std::{future::Future, io, sync::Arc, time::Duration};
use tokio::{
    net::TcpListener,
    sync::{Semaphore, broadcast, mpsc, oneshot},
    task::JoinSet,
    time::Instant,
};
use tokio_util::{sync::CancellationToken, task::TaskTracker};

// Hyper executes H2 streams outside the connection future, so both task layers
// must observe the server's force-cancellation signal.
#[derive(Clone)]
struct TransportExecutor {
    cancellation: CancellationToken,
    tasks: TaskTracker,
}

impl TransportExecutor {
    fn new(cancellation: CancellationToken) -> Self {
        Self {
            cancellation,
            tasks: TaskTracker::new(),
        }
    }

    fn cancel(&self) {
        self.cancellation.cancel();
    }

    fn close(&self) {
        self.tasks.close();
    }

    async fn close_and_wait(&self) {
        self.close();
        self.tasks.wait().await;
    }
}

impl<F> Executor<F> for TransportExecutor
where
    F: Future + Send + 'static,
    F::Output: Send + 'static,
{
    fn execute(&self, future: F) {
        let cancellation = self.cancellation.clone();
        let _task = self.tasks.spawn(async move {
            tokio::select! {
                biased;
                () = cancellation.cancelled_owned() => {}
                _ = future => {}
            }
        });
    }
}

pub(crate) struct TransportConfig {
    pub max_connections: usize,
    pub max_headers: usize,
    pub max_request_head_bytes: usize,
    pub http1_header_read_timeout: Duration,
    pub http2_max_concurrent_streams: u32,
    pub http2_keep_alive_interval: Option<Duration>,
    pub http2_keep_alive_timeout: Duration,
}

pub(crate) struct DrainCommand {
    pub deadline: Instant,
    pub listener_closed: oneshot::Sender<()>,
}

pub(crate) struct AcceptOutcome {
    pub fatal_error: Option<io::Error>,
    pub deadline_exceeded: bool,
}

pub(crate) async fn run(
    listener: TcpListener,
    app: HttpApp,
    config: TransportConfig,
    mut drain: mpsc::UnboundedReceiver<DrainCommand>,
    force_cancel: CancellationToken,
    fatal: mpsc::UnboundedSender<io::Error>,
    completion: oneshot::Sender<AcceptOutcome>,
) {
    let outcome = run_inner(listener, app, config, &mut drain, force_cancel, fatal).await;
    let _ = completion.send(outcome);
}

async fn run_inner(
    listener: TcpListener,
    app: HttpApp,
    config: TransportConfig,
    drain: &mut mpsc::UnboundedReceiver<DrainCommand>,
    force_cancel: CancellationToken,
    fatal: mpsc::UnboundedSender<io::Error>,
) -> AcceptOutcome {
    let executor = TransportExecutor::new(force_cancel.clone());
    let builder = Arc::new(connection_builder(&config, executor.clone()));
    let connections = Arc::new(Semaphore::new(config.max_connections));
    let (graceful, _) = broadcast::channel::<()>(1);
    let mut tasks = JoinSet::new();
    let mut accept_failures = AcceptFailureTracker::default();
    let mut fatal_error = None;
    let drain_command = loop {
        tokio::select! {
            biased;
            command = drain.recv() => break command,
            result = tasks.join_next(), if !tasks.is_empty() => {
                if let Some(Err(error)) = result {
                    tracing::error!(?error, "HTTP connection task panicked");
                }
            }
            accepted = listener.accept() => match accepted {
                Ok((stream, _peer)) => {
                    accept_failures.record_success();
                    let Ok(permit) = connections.clone().try_acquire_owned() else {
                        tracing::warn!("TCP connection limit reached; dropping accepted socket");
                        drop(stream);
                        continue;
                    };
                    let mut shutdown = graceful.subscribe();
                    let app = app.clone();
                    let builder = builder.clone();
                    tasks.spawn(async move {
                        let _permit = permit;
                        let connection = builder.serve_connection_with_upgrades(TokioIo::new(stream), app);
                        tokio::pin!(connection);
                        tokio::select! {
                            result = &mut connection => {
                                if let Err(error) = result {
                                    tracing::debug!(?error, "HTTP connection closed with protocol error");
                                }
                            }
                            _ = shutdown.recv() => {
                                connection.as_mut().graceful_shutdown();
                                if let Err(error) = connection.await {
                                    tracing::debug!(?error, "HTTP graceful connection drain failed");
                                }
                            }
                        }
                    });
                }
                Err(error) => match accept_failures.record_failure(error.kind()) {
                    AcceptFailureAction::RetryImmediately => continue,
                    AcceptFailureAction::Fatal => {
                        fatal_error = Some(error);
                        break None;
                    }
                    AcceptFailureAction::RetryAfter {
                        delay,
                        consecutive_failures,
                    } => {
                        tracing::warn!(?error, ?delay, consecutive_failures, "recoverable accept failure");
                        tokio::select! {
                            biased;
                            command = drain.recv() => break command,
                            () = tokio::time::sleep(delay) => {}
                        }
                    }
                },
            }
        }
    };

    drop(listener);
    let _ = graceful.send(());
    drop(graceful);

    let command = match (drain_command, fatal_error.as_ref()) {
        (Some(command), _) => Some(command),
        (None, Some(error)) => {
            let cloned = io::Error::new(error.kind(), error.to_string());
            let _ = fatal.send(cloned);
            drain.recv().await
        }
        (None, None) => None,
    };
    let Some(command) = command else {
        executor.cancel();
        tasks.abort_all();
        executor.close();
        return AcceptOutcome {
            fatal_error,
            deadline_exceeded: true,
        };
    };
    let DrainCommand {
        deadline,
        listener_closed,
    } = command;
    let _ = listener_closed.send(());

    let graceful_drain = async {
        drain_connection_tasks(&mut tasks).await;
        executor.close_and_wait().await;
    };
    let drained = tokio::select! {
        biased;
        () = force_cancel.cancelled() => false,
        result = tokio::time::timeout_at(deadline, graceful_drain) => result.is_ok(),
    };
    if !drained {
        executor.cancel();
        tasks.abort_all();
        // The deadline path stays bounded; cancelled tracked tasks reap themselves.
        executor.close();
    }
    AcceptOutcome {
        fatal_error,
        deadline_exceeded: !drained,
    }
}

async fn drain_connection_tasks(tasks: &mut JoinSet<()>) {
    while let Some(result) = tasks.join_next().await {
        if let Err(error) = result {
            tracing::error!(?error, "HTTP connection task failed while draining");
        }
    }
}

fn connection_builder(
    config: &TransportConfig,
    executor: TransportExecutor,
) -> Builder<TransportExecutor> {
    let mut builder = Builder::new(executor);
    builder
        .http1()
        .keep_alive(true)
        .writev(true)
        .max_headers(config.max_headers)
        .max_buf_size(config.max_request_head_bytes.max(8 * 1024))
        .header_read_timeout(Some(config.http1_header_read_timeout))
        .timer(TokioTimer::new());
    builder
        .http2()
        .max_header_list_size(config.max_request_head_bytes.min(u32::MAX as usize) as u32)
        .max_concurrent_streams(config.http2_max_concurrent_streams)
        .keep_alive_interval(config.http2_keep_alive_interval)
        .keep_alive_timeout(config.http2_keep_alive_timeout)
        .timer(TokioTimer::new());
    builder
}

fn accept_backoff(consecutive_failures: u8) -> Duration {
    let exponent = u32::from(consecutive_failures.saturating_sub(1).min(7));
    Duration::from_millis((10u64.saturating_mul(1u64 << exponent)).min(1_000))
}

#[derive(Default)]
struct AcceptFailureTracker {
    consecutive_failures: u8,
}

enum AcceptFailureAction {
    RetryImmediately,
    RetryAfter {
        delay: Duration,
        consecutive_failures: u8,
    },
    Fatal,
}

impl AcceptFailureTracker {
    fn record_success(&mut self) {
        self.consecutive_failures = 0;
    }

    fn record_failure(&mut self, kind: io::ErrorKind) -> AcceptFailureAction {
        if kind == io::ErrorKind::Interrupted {
            return AcceptFailureAction::RetryImmediately;
        }
        self.consecutive_failures = self.consecutive_failures.saturating_add(1);
        if self.consecutive_failures >= 16 {
            return AcceptFailureAction::Fatal;
        }
        AcceptFailureAction::RetryAfter {
            delay: accept_backoff(self.consecutive_failures),
            consecutive_failures: self.consecutive_failures,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        runtime::budget::ByteBudget,
        wire::{GuardedBody, GuardedChunk},
    };
    use bytes::Bytes;
    use http::{Request, Response};
    use http_body_util::Empty;
    use hyper::{
        body::{Body, Frame, Incoming, SizeHint},
        service::service_fn,
    };
    use std::{
        convert::Infallible,
        pin::Pin,
        sync::atomic::{AtomicBool, Ordering},
        task::{Context, Poll},
    };
    use tokio::io::AsyncWriteExt;

    struct DropProbe(Arc<AtomicBool>);

    impl Drop for DropProbe {
        fn drop(&mut self) {
            self.0.store(true, Ordering::Release);
        }
    }

    struct DropNotifyingBody {
        inner: GuardedBody,
        dropped: mpsc::UnboundedSender<()>,
    }

    impl Drop for DropNotifyingBody {
        fn drop(&mut self) {
            let _ = self.dropped.send(());
        }
    }

    impl Body for DropNotifyingBody {
        type Data = GuardedChunk;
        type Error = Infallible;

        fn poll_frame(
            mut self: Pin<&mut Self>,
            context: &mut Context<'_>,
        ) -> Poll<Option<Result<Frame<Self::Data>, Self::Error>>> {
            Pin::new(&mut self.inner).poll_frame(context)
        }

        fn is_end_stream(&self) -> bool {
            self.inner.is_end_stream()
        }

        fn size_hint(&self) -> SizeHint {
            self.inner.size_hint()
        }
    }

    fn test_transport_config() -> TransportConfig {
        TransportConfig {
            max_connections: 1,
            max_headers: 64,
            max_request_head_bytes: 32 * 1024,
            http1_header_read_timeout: Duration::from_secs(1),
            http2_max_concurrent_streams: 1,
            http2_keep_alive_interval: None,
            http2_keep_alive_timeout: Duration::from_secs(1),
        }
    }

    fn response_body(
        budget: &Arc<ByteBudget>,
        length: usize,
        dropped: mpsc::UnboundedSender<()>,
    ) -> DropNotifyingBody {
        let permit = Arc::new(budget.try_reserve(length).unwrap());
        DropNotifyingBody {
            inner: GuardedBody::new(Bytes::from(vec![b'x'; length]), Some(permit)),
            dropped,
        }
    }

    #[tokio::test]
    async fn transport_executor_drops_cancelled_tasks_including_late_spawns() {
        let executor = TransportExecutor::new(CancellationToken::new());
        let dropped = Arc::new(AtomicBool::new(false));
        let task_dropped = dropped.clone();
        let (entered, entered_receiver) = oneshot::channel();
        executor.execute(async move {
            let _probe = DropProbe(task_dropped);
            let _ = entered.send(());
            std::future::pending::<()>().await;
        });
        entered_receiver.await.unwrap();

        executor.cancel();
        executor.close_and_wait().await;

        assert!(dropped.load(Ordering::Acquire));
        assert!(executor.tasks.is_empty());

        let late_dropped = Arc::new(AtomicBool::new(false));
        let late_probe = DropProbe(late_dropped.clone());
        executor.execute(async move {
            let _probe = late_probe;
            std::future::pending::<()>().await;
        });
        executor.close_and_wait().await;

        assert!(late_dropped.load(Ordering::Acquire));
        assert!(executor.tasks.is_empty());
    }

    #[tokio::test]
    async fn h2_flow_control_holds_response_budget_until_transport_cancels() {
        const BODY_LENGTH: usize = 128 * 1024;
        const FLOW_WINDOW: u32 = 1024;

        let budget = ByteBudget::new(BODY_LENGTH);
        let service_budget = budget.clone();
        let (body_dropped, mut body_drops) = mpsc::unbounded_channel();
        let (client_io, server_io) = tokio::io::duplex(8 * 1024);
        let server_connection = tokio::spawn(async move {
            let service = service_fn(move |_request: Request<Incoming>| {
                let budget = service_budget.clone();
                let body_dropped = body_dropped.clone();
                async move {
                    Ok::<_, Infallible>(Response::new(response_body(
                        &budget,
                        BODY_LENGTH,
                        body_dropped,
                    )))
                }
            });
            connection_builder(
                &test_transport_config(),
                TransportExecutor::new(CancellationToken::new()),
            )
            .serve_connection(TokioIo::new(server_io), service)
            .await
        });

        let mut builder = hyper::client::conn::http2::Builder::new(TokioExecutor::new());
        builder
            .initial_stream_window_size(FLOW_WINDOW)
            .initial_connection_window_size(FLOW_WINDOW);
        let (mut sender, connection) = builder
            .handshake::<_, Empty<Bytes>>(TokioIo::new(client_io))
            .await
            .unwrap();
        let client_connection = tokio::spawn(async move {
            let _ = connection.await;
        });
        let request = Request::builder()
            .uri("http://localhost/flow-controlled")
            .body(Empty::new())
            .unwrap();
        let response = sender.send_request(request).await.unwrap();

        tokio::time::timeout(Duration::from_secs(1), body_drops.recv())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(budget.used(), BODY_LENGTH);
        drop(response);
        drop(sender);
        client_connection.abort();
        let _ = client_connection.await;
        let connection_result = tokio::time::timeout(Duration::from_secs(1), server_connection)
            .await
            .unwrap()
            .unwrap();
        drop(connection_result);
        assert_eq!(budget.used(), 0);
    }

    #[tokio::test]
    async fn h1_blocked_vectored_writer_holds_response_budget() {
        const BODY_LENGTH: usize = 32 * 1024;

        let budget = ByteBudget::new(BODY_LENGTH);
        let service_budget = budget.clone();
        let (body_dropped, mut body_drops) = mpsc::unbounded_channel();
        let (mut client_io, server_io) = tokio::io::duplex(128);
        let server_connection = tokio::spawn(async move {
            let service = service_fn(move |_request: Request<Incoming>| {
                let budget = service_budget.clone();
                let body_dropped = body_dropped.clone();
                async move {
                    Ok::<_, Infallible>(Response::new(response_body(
                        &budget,
                        BODY_LENGTH,
                        body_dropped,
                    )))
                }
            });
            connection_builder(
                &test_transport_config(),
                TransportExecutor::new(CancellationToken::new()),
            )
            .serve_connection(TokioIo::new(server_io), service)
            .await
        });

        client_io
            .write_all(b"GET /blocked HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")
            .await
            .unwrap();
        tokio::time::timeout(Duration::from_secs(1), body_drops.recv())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(budget.used(), BODY_LENGTH);

        drop(client_io);
        let connection_result = tokio::time::timeout(Duration::from_secs(1), server_connection)
            .await
            .unwrap()
            .unwrap();
        drop(connection_result);
        assert_eq!(budget.used(), 0);
    }

    #[test]
    fn accept_backoff_starts_at_ten_ms_and_caps_at_one_second() {
        assert_eq!(accept_backoff(1), Duration::from_millis(10));
        assert_eq!(accept_backoff(2), Duration::from_millis(20));
        assert_eq!(accept_backoff(16), Duration::from_secs(1));
    }

    #[test]
    fn accept_failure_tracking_retries_interrupted_and_resets_after_success() {
        let mut tracker = AcceptFailureTracker::default();
        assert!(matches!(
            tracker.record_failure(io::ErrorKind::Interrupted),
            AcceptFailureAction::RetryImmediately
        ));
        assert!(matches!(
            tracker.record_failure(io::ErrorKind::ConnectionAborted),
            AcceptFailureAction::RetryAfter {
                consecutive_failures: 1,
                delay,
            } if delay == Duration::from_millis(10)
        ));
        tracker.record_success();
        assert!(matches!(
            tracker.record_failure(io::ErrorKind::ConnectionAborted),
            AcceptFailureAction::RetryAfter {
                consecutive_failures: 1,
                delay,
            } if delay == Duration::from_millis(10)
        ));
    }

    #[test]
    fn sixteenth_recoverable_accept_failure_is_fatal() {
        let mut tracker = AcceptFailureTracker::default();
        for expected in 1..16 {
            assert!(matches!(
                tracker.record_failure(io::ErrorKind::ConnectionAborted),
                AcceptFailureAction::RetryAfter {
                    consecutive_failures,
                    ..
                } if consecutive_failures == expected
            ));
        }
        assert!(matches!(
            tracker.record_failure(io::ErrorKind::ConnectionAborted),
            AcceptFailureAction::Fatal
        ));
    }
}
