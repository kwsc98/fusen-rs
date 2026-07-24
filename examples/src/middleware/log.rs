use fusen_rs::{InvocationFinish, InvocationObserver, InvocationStart};
use tracing::info;

pub struct LogObserver;

impl InvocationObserver for LogObserver {
    fn on_start(&self, _event: &InvocationStart<'_>) {}

    fn on_finish(&self, event: &InvocationFinish<'_>) {
        info!(
            side = ?event.side,
            request_id = event.request_id,
            service = event.service,
            method = event.method,
            phase = ?event.phase,
            outcome = ?event.outcome,
            status = event.http_status.map(|status| status.as_u16()),
            error_code = event.error_code,
            elapsed_ms = event.elapsed.as_millis(),
            "request completed"
        );
    }
}
