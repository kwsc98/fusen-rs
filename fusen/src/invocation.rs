use crate::error::FusenError;
use http::StatusCode;
use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicU8, Ordering},
    },
    time::{Duration, Instant},
};

/// Identifies which side of an RPC emitted a lifecycle event.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InvocationSide {
    /// A generated or low-level RPC client invocation.
    Client,
    /// An inbound server request.
    Server,
}

/// The most recent stage reached by an invocation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum InvocationPhase {
    /// Initial client state checks or server concurrency admission.
    Admission,
    /// Client method lookup and request construction.
    BuildRequest,
    /// HTTP request or response body decoding.
    Decode,
    /// Server route matching.
    Route,
    /// Client routing, policy, and service instance selection.
    Cluster,
    /// User middleware execution.
    Middleware,
    /// Client HTTP transport execution.
    Transport,
    /// Server RPC method dispatch.
    Service,
    /// Server HTTP response encoding.
    Encode,
    /// The invocation produced a complete response.
    Complete,
}

impl InvocationPhase {
    fn from_u8(value: u8) -> Self {
        match value {
            1 => Self::BuildRequest,
            2 => Self::Decode,
            3 => Self::Route,
            4 => Self::Cluster,
            5 => Self::Middleware,
            6 => Self::Transport,
            7 => Self::Service,
            8 => Self::Encode,
            9 => Self::Complete,
            _ => Self::Admission,
        }
    }
}

/// Terminal lifecycle status observed for one invocation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InvocationOutcome {
    /// A successful response was produced.
    Success,
    /// The invocation returned a framework, application, or HTTP error.
    Error,
    /// The absolute invocation deadline elapsed.
    Timeout,
    /// The caller or connection dropped the invocation Future.
    Cancelled,
}

/// Borrowed information emitted when an invocation starts.
#[derive(Debug)]
pub struct InvocationStart<'a> {
    /// Client or server side.
    pub side: InvocationSide,
    /// Correlation ID assigned to the invocation.
    pub request_id: &'a str,
    /// Service ID when known at start time.
    pub service: Option<&'a str>,
    /// RPC method name when known at start time.
    pub method: Option<&'a str>,
}

/// Borrowed information emitted exactly once when an invocation finishes or is cancelled.
#[derive(Debug)]
pub struct InvocationFinish<'a> {
    /// Client or server side.
    pub side: InvocationSide,
    /// Correlation ID assigned to the invocation.
    pub request_id: &'a str,
    /// Routed service ID, when available.
    pub service: Option<&'a str>,
    /// Routed RPC method, when available.
    pub method: Option<&'a str>,
    /// Last stage reached before completion.
    pub phase: InvocationPhase,
    /// Terminal lifecycle outcome.
    pub outcome: InvocationOutcome,
    /// Wall-clock time from invocation admission to completion.
    pub elapsed: Duration,
    /// HTTP status produced or associated with an error.
    pub http_status: Option<StatusCode>,
    /// Stable framework or application error code.
    pub error_code: Option<&'a str>,
}

/// Synchronous observer for complete client or server invocation lifecycles.
///
/// Implementations run on the request task and must not block or panic. Events intentionally omit
/// request bodies, credentials, and complete HTTP headers.
pub trait InvocationObserver: Send + Sync {
    /// Observes invocation admission. Server routing information may not be known yet.
    fn on_start(&self, event: &InvocationStart<'_>);
    /// Observes the invocation's single terminal lifecycle result.
    fn on_finish(&self, event: &InvocationFinish<'_>);
}

#[derive(Clone)]
pub(crate) struct PhaseTracker(Option<Arc<AtomicU8>>);

impl PhaseTracker {
    fn new(enabled: bool) -> Self {
        Self(enabled.then(|| Arc::new(AtomicU8::new(InvocationPhase::Admission as u8))))
    }

    pub(crate) fn set(&self, phase: InvocationPhase) {
        if let Some(value) = &self.0 {
            value.store(phase as u8, Ordering::Release);
        }
    }

    fn get(&self) -> InvocationPhase {
        self.0
            .as_ref()
            .map(|value| InvocationPhase::from_u8(value.load(Ordering::Acquire)))
            .unwrap_or(InvocationPhase::Admission)
    }
}

#[derive(Clone, Default)]
pub(crate) struct TargetTracker(Option<Arc<Mutex<InvocationTarget>>>);

#[derive(Default)]
struct InvocationTarget {
    service: Option<String>,
    method: Option<String>,
}

impl TargetTracker {
    fn new(enabled: bool, service: Option<&str>, method: Option<&str>) -> Self {
        Self(enabled.then(|| {
            Arc::new(Mutex::new(InvocationTarget {
                service: service.map(str::to_owned),
                method: method.map(str::to_owned),
            }))
        }))
    }

    pub(crate) fn set(&self, service: &str, method: &str) {
        if let Some(target) = &self.0 {
            let mut target = target.lock().expect("invocation target lock poisoned");
            target.service = Some(service.to_owned());
            target.method = Some(method.to_owned());
        }
    }
}

pub(crate) struct InvocationGuard<'a> {
    observers: &'a [Arc<dyn InvocationObserver>],
    side: InvocationSide,
    request_id: Option<String>,
    target: TargetTracker,
    started: Option<Instant>,
    phase: PhaseTracker,
    finished: bool,
}

impl<'a> InvocationGuard<'a> {
    pub(crate) fn start(
        observers: &'a [Arc<dyn InvocationObserver>],
        side: InvocationSide,
        request_id: &str,
        service: Option<&str>,
        method: Option<&str>,
    ) -> Self {
        let enabled = !observers.is_empty();
        let guard = Self {
            observers,
            side,
            request_id: enabled.then(|| request_id.to_owned()),
            target: TargetTracker::new(enabled, service, method),
            started: enabled.then(Instant::now),
            phase: PhaseTracker::new(enabled),
            finished: false,
        };
        if let Some(target) = &guard.target.0 {
            let (service, method) = {
                let target = target.lock().expect("invocation target lock poisoned");
                (target.service.clone(), target.method.clone())
            };
            let event = InvocationStart {
                side: guard.side,
                request_id: guard
                    .request_id
                    .as_deref()
                    .expect("observed invocation request ID is missing"),
                service: service.as_deref(),
                method: method.as_deref(),
            };
            for observer in guard.observers.iter() {
                observer.on_start(&event);
            }
        }
        guard
    }

    pub(crate) fn tracker(&self) -> PhaseTracker {
        self.phase.clone()
    }

    pub(crate) fn target_tracker(&self) -> TargetTracker {
        self.target.clone()
    }

    pub(crate) fn finish_response(&mut self, status: StatusCode) {
        let (outcome, code) = if status.is_success() {
            (InvocationOutcome::Success, None)
        } else {
            (InvocationOutcome::Error, Some("http_error"))
        };
        self.finish(outcome, Some(status), code, InvocationPhase::Complete);
    }

    pub(crate) fn finish_error(&mut self, error: &FusenError) {
        self.finish(
            InvocationOutcome::Error,
            Some(error.status()),
            Some(error.code()),
            self.phase.get(),
        );
    }

    pub(crate) fn finish_timeout(&mut self) {
        self.finish(
            InvocationOutcome::Timeout,
            Some(StatusCode::GATEWAY_TIMEOUT),
            Some("timeout"),
            self.phase.get(),
        );
    }

    fn finish(
        &mut self,
        outcome: InvocationOutcome,
        http_status: Option<StatusCode>,
        error_code: Option<&str>,
        phase: InvocationPhase,
    ) {
        if self.finished {
            return;
        }
        self.finished = true;
        if self.observers.is_empty() {
            return;
        }
        let (service, method) = {
            let target = self
                .target
                .0
                .as_ref()
                .expect("observed invocation target is missing")
                .lock()
                .expect("invocation target lock poisoned");
            (target.service.clone(), target.method.clone())
        };
        let event = InvocationFinish {
            side: self.side,
            request_id: self
                .request_id
                .as_deref()
                .expect("observed invocation request ID is missing"),
            service: service.as_deref(),
            method: method.as_deref(),
            phase,
            outcome,
            elapsed: self
                .started
                .expect("observed invocation start time is missing")
                .elapsed(),
            http_status,
            error_code,
        };
        for observer in self.observers.iter() {
            observer.on_finish(&event);
        }
    }
}

impl Drop for InvocationGuard<'_> {
    fn drop(&mut self) {
        self.finish(InvocationOutcome::Cancelled, None, None, self.phase.get());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    #[derive(Default)]
    struct RecordingObserver(Mutex<Vec<InvocationOutcome>>);

    impl InvocationObserver for RecordingObserver {
        fn on_start(&self, _event: &InvocationStart<'_>) {}

        fn on_finish(&self, event: &InvocationFinish<'_>) {
            self.0.lock().unwrap().push(event.outcome);
        }
    }

    #[test]
    fn guard_reports_exactly_one_completion() {
        let observer = Arc::new(RecordingObserver::default());
        let observers: Arc<[Arc<dyn InvocationObserver>]> =
            Arc::from(vec![observer.clone() as Arc<dyn InvocationObserver>]);
        let mut guard = InvocationGuard::start(
            &observers,
            InvocationSide::Client,
            "request",
            Some("service"),
            Some("method"),
        );
        guard.finish_response(StatusCode::OK);
        guard.finish_timeout();
        drop(guard);
        assert_eq!(*observer.0.lock().unwrap(), [InvocationOutcome::Success]);
    }

    #[test]
    fn dropping_unfinished_guard_reports_cancellation() {
        let observer = Arc::new(RecordingObserver::default());
        let observers: Arc<[Arc<dyn InvocationObserver>]> =
            Arc::from(vec![observer.clone() as Arc<dyn InvocationObserver>]);
        drop(InvocationGuard::start(
            &observers,
            InvocationSide::Server,
            "request",
            None,
            None,
        ));
        assert_eq!(*observer.0.lock().unwrap(), [InvocationOutcome::Cancelled]);
    }

    struct OrderedObserver {
        name: &'static str,
        events: Arc<Mutex<Vec<String>>>,
    }

    impl InvocationObserver for OrderedObserver {
        fn on_start(&self, _event: &InvocationStart<'_>) {
            self.events
                .lock()
                .unwrap()
                .push(format!("{}:start", self.name));
        }

        fn on_finish(&self, _event: &InvocationFinish<'_>) {
            self.events
                .lock()
                .unwrap()
                .push(format!("{}:finish", self.name));
        }
    }

    #[test]
    fn observers_are_notified_in_registration_order() {
        let events = Arc::new(Mutex::new(Vec::new()));
        let observers: Arc<[Arc<dyn InvocationObserver>]> = Arc::from(vec![
            Arc::new(OrderedObserver {
                name: "first",
                events: events.clone(),
            }) as Arc<dyn InvocationObserver>,
            Arc::new(OrderedObserver {
                name: "second",
                events: events.clone(),
            }),
        ]);
        let mut guard = InvocationGuard::start(
            &observers,
            InvocationSide::Client,
            "request",
            Some("service"),
            Some("method"),
        );
        guard.finish_response(StatusCode::OK);

        assert_eq!(
            *events.lock().unwrap(),
            [
                "first:start",
                "second:start",
                "first:finish",
                "second:finish"
            ]
        );
    }
}
