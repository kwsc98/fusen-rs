use fusen_observability::{MetricEvent, MetricsRecorder};
use std::{
    panic::{AssertUnwindSafe, catch_unwind},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

#[derive(Clone, Default)]
pub(crate) struct SafeMetrics(Option<Arc<RecorderState>>);

struct RecorderState {
    recorder: Arc<dyn MetricsRecorder>,
    disabled: AtomicBool,
}

impl SafeMetrics {
    pub(crate) fn new(recorder: Option<Arc<dyn MetricsRecorder>>) -> Self {
        Self(recorder.map(|recorder| {
            Arc::new(RecorderState {
                recorder,
                disabled: AtomicBool::new(false),
            })
        }))
    }

    pub(crate) fn record(&self, event: &MetricEvent<'_>) {
        let Some(state) = &self.0 else {
            return;
        };
        if state.disabled.load(Ordering::Acquire) {
            return;
        }
        if catch_unwind(AssertUnwindSafe(|| state.recorder.record(event))).is_err() {
            state.disabled.store(true, Ordering::Release);
            tracing::error!("metrics recorder panicked and was disabled");
        }
    }

    #[cfg(test)]
    fn disabled(&self) -> bool {
        self.0
            .as_ref()
            .is_some_and(|state| state.disabled.load(Ordering::Acquire))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use fusen_observability::{AdmissionRejectedEvent, MetricSide, MetricsRecorder};
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct Panics(AtomicUsize);

    impl MetricsRecorder for Panics {
        fn record(&self, _event: &MetricEvent<'_>) {
            self.0.fetch_add(1, Ordering::SeqCst);
            panic!("boom");
        }
    }

    #[test]
    fn first_panic_disables_the_recorder() {
        let recorder = Arc::new(Panics(AtomicUsize::new(0)));
        let metrics = SafeMetrics::new(Some(recorder.clone()));
        let event = MetricEvent::AdmissionRejected(AdmissionRejectedEvent::new(
            MetricSide::Client,
            "concurrency",
        ));
        metrics.record(&event);
        metrics.record(&event);
        assert!(metrics.disabled());
        assert_eq!(recorder.0.load(Ordering::SeqCst), 1);
    }
}
