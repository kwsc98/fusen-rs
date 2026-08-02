//! Rolling-window endpoint and service circuit breakers.

use std::{
    collections::HashMap,
    fmt,
    hash::Hash,
    panic::{AssertUnwindSafe, catch_unwind},
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::time::Instant;

const HARD_MAX_OPEN_DURATION: Duration = Duration::from_secs(120);
pub(crate) const DEFAULT_ENDPOINT_IDLE_EVICTION: Duration = Duration::from_secs(600);

/// Stable policy-oriented classification of one failed physical attempt.
///
/// Runtime error types are intentionally absent from this contract. The invocation layer maps its
/// typed errors into one of these categories before consulting retry or breaker policy.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum FailureClass {
    /// Establishing a connection to the selected endpoint failed.
    Connect,
    /// An established transport failed before a valid response completed.
    Transport,
    /// The physical attempt exceeded its bounded transport or response deadline.
    Timeout,
    /// The selected endpoint or remote service reported temporary unavailability.
    Unavailable,
    /// The remote service reported transient overload or throttling.
    Overloaded,
    /// A retryable remote server failure was returned.
    RemoteServer,
    /// A remote server failure affects endpoint health but is not eligible for retry.
    RemoteFailure,
    /// The peer returned a malformed or protocol-invalid response.
    Protocol,
    /// A valid application response represented an application-level failure.
    Application,
    /// Local validation rejected the invocation before transport work began.
    InvalidRequest,
    /// The caller cancelled the invocation.
    Cancelled,
    /// Local admission or another local policy rejected the attempt before transport work began.
    LocalRejection,
}

impl FailureClass {
    /// Returns whether the built-in policy may retry this failure for a replayable method.
    pub const fn is_retryable(self) -> bool {
        matches!(
            self,
            Self::Connect
                | Self::Transport
                | Self::Timeout
                | Self::Unavailable
                | Self::Overloaded
                | Self::RemoteServer
        )
    }

    const fn breaker_impact(self) -> BreakerImpact {
        match self {
            Self::Connect
            | Self::Transport
            | Self::Timeout
            | Self::Unavailable
            | Self::RemoteServer
            | Self::RemoteFailure
            | Self::Protocol => BreakerImpact::Failure,
            Self::Application => BreakerImpact::Success,
            Self::Overloaded | Self::InvalidRequest | Self::Cancelled | Self::LocalRejection => {
                BreakerImpact::Ignore
            }
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum BreakerImpact {
    Success,
    Failure,
    Ignore,
    Unattempted,
}

#[derive(Clone, Debug)]
pub(crate) struct BreakerConfig {
    window: Duration,
    buckets: u8,
    minimum_samples: u32,
    failure_ratio: f64,
    open_duration: Duration,
    max_open_duration: Duration,
    half_open_concurrency: u32,
    close_successes: u32,
}

impl BreakerConfig {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(
        window: Duration,
        buckets: u8,
        minimum_samples: u32,
        failure_ratio: f64,
        open_duration: Duration,
        max_open_duration: Duration,
        half_open_concurrency: u32,
        close_successes: u32,
    ) -> Self {
        debug_assert!(!window.is_zero());
        debug_assert!(buckets > 0);
        debug_assert!(minimum_samples > 0);
        debug_assert!((0.0..=1.0).contains(&failure_ratio));
        debug_assert!(!open_duration.is_zero());
        debug_assert!(!max_open_duration.is_zero());
        debug_assert!(half_open_concurrency > 0);
        debug_assert!(close_successes > 0);
        Self {
            window,
            buckets,
            minimum_samples,
            failure_ratio,
            open_duration,
            max_open_duration: max_open_duration.min(HARD_MAX_OPEN_DURATION),
            half_open_concurrency,
            close_successes,
        }
    }

    fn initial_open_duration(&self) -> Duration {
        self.open_duration.min(self.max_open_duration)
    }

    fn next_open_duration(&self, previous: Duration) -> Duration {
        previous
            .checked_mul(2)
            .unwrap_or(self.max_open_duration)
            .min(self.max_open_duration)
    }
}

#[cfg(test)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum BreakerState {
    Closed,
    Open,
    HalfOpen,
}

#[cfg(test)]
#[derive(Clone, Copy, Debug)]
pub(crate) struct BreakerSnapshot {
    pub(crate) state: BreakerState,
    pub(crate) samples: u64,
    pub(crate) failures: u64,
    pub(crate) retry_after: Option<Duration>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum BreakerRejection {
    Open { retry_after: Duration },
    HalfOpenSaturated,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum BreakerPhase {
    Closed,
    Open,
    HalfOpen,
}

type TransitionObserver = Arc<dyn Fn(BreakerPhase) + Send + Sync + 'static>;

pub(crate) struct CircuitBreaker {
    config: BreakerConfig,
    inner: Mutex<BreakerInner>,
    observer: Option<TransitionObserver>,
}

impl fmt::Debug for CircuitBreaker {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("CircuitBreaker")
            .field("config", &self.config)
            .field("inner", &self.inner)
            .finish_non_exhaustive()
    }
}

#[derive(Debug)]
struct BreakerInner {
    epoch: u64,
    state: MachineState,
}

#[derive(Debug)]
enum MachineState {
    Closed(RollingWindow),
    Open {
        until: Instant,
        duration: Duration,
    },
    HalfOpen {
        in_flight: u32,
        consecutive_successes: u32,
        previous_open_duration: Duration,
    },
}

impl CircuitBreaker {
    #[cfg(test)]
    pub(crate) fn endpoint(config: BreakerConfig) -> Arc<Self> {
        Self::new(config)
    }

    #[cfg(test)]
    pub(crate) fn service(config: BreakerConfig) -> Arc<Self> {
        Self::new(config)
    }

    #[cfg(test)]
    fn new(config: BreakerConfig) -> Arc<Self> {
        Self::with_observer(config, None)
    }

    pub(crate) fn observed(config: BreakerConfig, observer: TransitionObserver) -> Arc<Self> {
        Self::with_observer(config, Some(observer))
    }

    fn with_observer(config: BreakerConfig, observer: Option<TransitionObserver>) -> Arc<Self> {
        Arc::new(Self {
            inner: Mutex::new(BreakerInner {
                epoch: 0,
                state: MachineState::Closed(RollingWindow::new(&config, Instant::now())),
            }),
            config,
            observer,
        })
    }

    pub(crate) fn try_acquire(self: &Arc<Self>) -> Result<BreakerPermit, BreakerRejection> {
        let now = Instant::now();
        let mut inner = self.inner.lock().unwrap_or_else(|error| error.into_inner());
        let mut transition = None;
        loop {
            let epoch = inner.epoch;
            match &mut inner.state {
                MachineState::Closed(window) => {
                    window.advance(now);
                    let permit = BreakerPermit::new(
                        self.clone(),
                        PermitToken {
                            epoch,
                            phase: PermitPhase::Closed,
                        },
                    );
                    drop(inner);
                    self.notify(transition);
                    return Ok(permit);
                }
                MachineState::Open { until, duration } if now >= *until => {
                    let previous_open_duration = *duration;
                    inner.epoch = inner.epoch.wrapping_add(1);
                    inner.state = MachineState::HalfOpen {
                        in_flight: 0,
                        consecutive_successes: 0,
                        previous_open_duration,
                    };
                    transition = Some(BreakerPhase::HalfOpen);
                }
                MachineState::Open { until, .. } => {
                    let rejection = BreakerRejection::Open {
                        retry_after: until.saturating_duration_since(now),
                    };
                    drop(inner);
                    self.notify(transition);
                    return Err(rejection);
                }
                MachineState::HalfOpen { in_flight, .. }
                    if *in_flight >= self.config.half_open_concurrency =>
                {
                    drop(inner);
                    self.notify(transition);
                    return Err(BreakerRejection::HalfOpenSaturated);
                }
                MachineState::HalfOpen { in_flight, .. } => {
                    *in_flight += 1;
                    let permit = BreakerPermit::new(
                        self.clone(),
                        PermitToken {
                            epoch,
                            phase: PermitPhase::HalfOpen,
                        },
                    );
                    drop(inner);
                    self.notify(transition);
                    return Ok(permit);
                }
            }
        }
    }

    #[cfg(test)]
    pub(crate) fn snapshot(&self) -> BreakerSnapshot {
        let now = Instant::now();
        let mut inner = self.inner.lock().unwrap_or_else(|error| error.into_inner());
        let (state, samples, failures, retry_after) = match &mut inner.state {
            MachineState::Closed(window) => {
                window.advance(now);
                let counts = window.counts();
                (
                    BreakerState::Closed,
                    counts.samples(),
                    counts.failures,
                    None,
                )
            }
            MachineState::Open { until, .. } => (
                BreakerState::Open,
                0,
                0,
                Some(until.saturating_duration_since(now)),
            ),
            MachineState::HalfOpen { .. } => (BreakerState::HalfOpen, 0, 0, None),
        };
        BreakerSnapshot {
            state,
            samples,
            failures,
            retry_after,
        }
    }

    fn complete(&self, token: PermitToken, outcome: PermitOutcome) {
        let now = Instant::now();
        let impact = match outcome {
            PermitOutcome::Success => BreakerImpact::Success,
            PermitOutcome::Failure(failure) => failure.breaker_impact(),
            PermitOutcome::Abandoned => BreakerImpact::Ignore,
            PermitOutcome::Unattempted => BreakerImpact::Unattempted,
        };
        let mut inner = self.inner.lock().unwrap_or_else(|error| error.into_inner());
        if token.epoch != inner.epoch {
            return;
        }
        let transition = match token.phase {
            PermitPhase::Closed => self.complete_closed(&mut inner, now, impact),
            PermitPhase::HalfOpen => self.complete_half_open(&mut inner, now, impact),
        };
        drop(inner);
        self.notify(transition);
    }

    fn complete_closed(
        &self,
        inner: &mut BreakerInner,
        now: Instant,
        impact: BreakerImpact,
    ) -> Option<BreakerPhase> {
        let should_open = {
            let MachineState::Closed(window) = &mut inner.state else {
                return None;
            };
            window.advance(now);
            match impact {
                BreakerImpact::Success => window.record_success(),
                BreakerImpact::Failure => window.record_failure(),
                BreakerImpact::Ignore | BreakerImpact::Unattempted => return None,
            }
            impact == BreakerImpact::Failure && self.threshold_exceeded(window.counts())
        };
        if should_open {
            let duration = self.config.initial_open_duration();
            inner.epoch = inner.epoch.wrapping_add(1);
            inner.state = MachineState::Open {
                until: now + duration,
                duration,
            };
            Some(BreakerPhase::Open)
        } else {
            None
        }
    }

    fn complete_half_open(
        &self,
        inner: &mut BreakerInner,
        now: Instant,
        impact: BreakerImpact,
    ) -> Option<BreakerPhase> {
        enum Transition {
            None,
            Close,
            Reopen(Duration),
        }

        let transition = {
            let MachineState::HalfOpen {
                in_flight,
                consecutive_successes,
                previous_open_duration,
            } = &mut inner.state
            else {
                return None;
            };
            *in_flight = in_flight.saturating_sub(1);
            match impact {
                BreakerImpact::Success => {
                    *consecutive_successes = consecutive_successes.saturating_add(1);
                    if *consecutive_successes >= self.config.close_successes {
                        Transition::Close
                    } else {
                        Transition::None
                    }
                }
                BreakerImpact::Failure => {
                    Transition::Reopen(self.config.next_open_duration(*previous_open_duration))
                }
                BreakerImpact::Ignore => Transition::None,
                BreakerImpact::Unattempted => Transition::None,
            }
        };

        match transition {
            Transition::None => None,
            Transition::Close => {
                inner.epoch = inner.epoch.wrapping_add(1);
                inner.state = MachineState::Closed(RollingWindow::new(&self.config, now));
                Some(BreakerPhase::Closed)
            }
            Transition::Reopen(duration) => {
                inner.epoch = inner.epoch.wrapping_add(1);
                inner.state = MachineState::Open {
                    until: now + duration,
                    duration,
                };
                Some(BreakerPhase::Open)
            }
        }
    }

    fn notify(&self, transition: Option<BreakerPhase>) {
        let (Some(observer), Some(transition)) = (&self.observer, transition) else {
            return;
        };
        if catch_unwind(AssertUnwindSafe(|| observer(transition))).is_err() {
            tracing::error!("circuit-breaker transition observer panicked");
        }
    }

    fn threshold_exceeded(&self, counts: WindowCounts) -> bool {
        let samples = counts.samples();
        samples >= u64::from(self.config.minimum_samples)
            && counts.failures as f64 / samples as f64 >= self.config.failure_ratio
    }
}

#[derive(Clone, Copy, Debug)]
struct PermitToken {
    epoch: u64,
    phase: PermitPhase,
}

#[derive(Clone, Copy, Debug)]
enum PermitPhase {
    Closed,
    HalfOpen,
}

#[derive(Clone, Copy, Debug)]
enum PermitOutcome {
    Success,
    Failure(FailureClass),
    Abandoned,
    Unattempted,
}

#[derive(Debug)]
pub(crate) struct BreakerPermit {
    breaker: Arc<CircuitBreaker>,
    token: Option<PermitToken>,
}

impl BreakerPermit {
    fn new(breaker: Arc<CircuitBreaker>, token: PermitToken) -> Self {
        Self {
            breaker,
            token: Some(token),
        }
    }

    pub(crate) fn succeed(mut self) {
        self.finish(PermitOutcome::Success);
    }

    pub(crate) fn fail(mut self, failure: FailureClass) {
        self.finish(PermitOutcome::Failure(failure));
    }

    pub(crate) fn release_unattempted(mut self) {
        self.finish(PermitOutcome::Unattempted);
    }

    fn finish(&mut self, outcome: PermitOutcome) {
        if let Some(token) = self.token.take() {
            self.breaker.complete(token, outcome);
        }
    }
}

impl Drop for BreakerPermit {
    fn drop(&mut self) {
        self.finish(PermitOutcome::Abandoned);
    }
}

#[derive(Clone, Copy, Debug, Default)]
struct Bucket {
    successes: u32,
    failures: u32,
}

#[derive(Debug)]
struct RollingWindow {
    buckets: Box<[Bucket]>,
    current: usize,
    current_start: Instant,
    bucket_width: Duration,
}

impl RollingWindow {
    fn new(config: &BreakerConfig, now: Instant) -> Self {
        let bucket_width = config.window / u32::from(config.buckets);
        debug_assert!(!bucket_width.is_zero());
        Self {
            buckets: vec![Bucket::default(); usize::from(config.buckets)].into_boxed_slice(),
            current: 0,
            current_start: now,
            bucket_width: bucket_width.max(Duration::from_nanos(1)),
        }
    }

    fn advance(&mut self, now: Instant) {
        let width_nanos = self.bucket_width.as_nanos();
        let elapsed_nanos = now.saturating_duration_since(self.current_start).as_nanos();
        let steps = elapsed_nanos / width_nanos;
        if steps == 0 {
            return;
        }
        if steps >= self.buckets.len() as u128 {
            self.buckets.fill(Bucket::default());
            self.current = 0;
            self.current_start = now;
            return;
        }
        for _ in 0..steps {
            self.current = (self.current + 1) % self.buckets.len();
            self.buckets[self.current] = Bucket::default();
            self.current_start += self.bucket_width;
        }
    }

    fn record_success(&mut self) {
        self.buckets[self.current].successes =
            self.buckets[self.current].successes.saturating_add(1);
    }

    fn record_failure(&mut self) {
        self.buckets[self.current].failures = self.buckets[self.current].failures.saturating_add(1);
    }

    fn counts(&self) -> WindowCounts {
        self.buckets.iter().fold(
            WindowCounts {
                successes: 0,
                failures: 0,
            },
            |counts, bucket| WindowCounts {
                successes: counts.successes + u64::from(bucket.successes),
                failures: counts.failures + u64::from(bucket.failures),
            },
        )
    }
}

#[derive(Clone, Copy, Debug)]
struct WindowCounts {
    successes: u64,
    failures: u64,
}

impl WindowCounts {
    const fn samples(self) -> u64 {
        self.successes + self.failures
    }
}

#[derive(Debug)]
struct EndpointEntry {
    breaker: Arc<CircuitBreaker>,
    last_used: Instant,
}

/// Bounded endpoint-breaker map with opportunistic LRU and idle eviction.
#[derive(Debug)]
pub(crate) struct EndpointBreakerStore<K> {
    config: BreakerConfig,
    max_entries: usize,
    idle_eviction: Duration,
    entries: Mutex<HashMap<K, EndpointEntry>>,
}

impl<K> EndpointBreakerStore<K>
where
    K: Clone + Eq + Hash,
{
    pub(crate) fn new(config: BreakerConfig, max_entries: usize, idle_eviction: Duration) -> Self {
        debug_assert!(max_entries > 0);
        debug_assert!(!idle_eviction.is_zero());
        Self {
            config,
            max_entries,
            idle_eviction,
            entries: Mutex::new(HashMap::new()),
        }
    }

    #[cfg(test)]
    pub(crate) fn get_or_insert(&self, key: K) -> Arc<CircuitBreaker> {
        self.get_or_insert_with_observer(key, None)
    }

    pub(crate) fn get_or_insert_observed(
        &self,
        key: K,
        observer: TransitionObserver,
    ) -> Arc<CircuitBreaker> {
        self.get_or_insert_with_observer(key, Some(observer))
    }

    /// Creates an observed breaker without retaining it in the endpoint cache.
    ///
    /// Discovery uses this for an invocation that still holds an older directory snapshot after
    /// the endpoint has been removed from the active membership. The in-flight invocation keeps
    /// normal breaker semantics without resurrecting an absent endpoint in the bounded store.
    pub(crate) fn untracked_observed(&self, observer: TransitionObserver) -> Arc<CircuitBreaker> {
        CircuitBreaker::with_observer(self.config.clone(), Some(observer))
    }

    /// Retains cached entries selected by `keep` while existing permits retain their Arcs.
    pub(crate) fn retain(&self, mut keep: impl FnMut(&K) -> bool) {
        self.entries
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .retain(|key, _| keep(key));
    }

    fn get_or_insert_with_observer(
        &self,
        key: K,
        observer: Option<TransitionObserver>,
    ) -> Arc<CircuitBreaker> {
        let now = Instant::now();
        let mut entries = self
            .entries
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        Self::evict_idle_locked(&mut entries, now, self.idle_eviction);
        if let Some(entry) = entries.get_mut(&key) {
            entry.last_used = now;
            return entry.breaker.clone();
        }
        if entries.len() >= self.max_entries
            && let Some(oldest) = entries
                .iter()
                .min_by_key(|(_, entry)| entry.last_used)
                .map(|(key, _)| key.clone())
        {
            entries.remove(&oldest);
        }

        let breaker = CircuitBreaker::with_observer(self.config.clone(), observer);
        entries.insert(
            key,
            EndpointEntry {
                breaker: breaker.clone(),
                last_used: now,
            },
        );
        breaker
    }

    #[cfg(test)]
    pub(crate) fn evict_idle(&self) -> usize {
        let mut entries = self
            .entries
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        Self::evict_idle_locked(&mut entries, Instant::now(), self.idle_eviction)
    }

    #[cfg(test)]
    pub(crate) fn len(&self) -> usize {
        self.entries
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .len()
    }

    #[cfg(test)]
    pub(crate) fn is_empty(&self) -> bool {
        self.len() == 0
    }

    fn evict_idle_locked(
        entries: &mut HashMap<K, EndpointEntry>,
        now: Instant,
        idle_eviction: Duration,
    ) -> usize {
        let previous = entries.len();
        entries.retain(|_, entry| now.saturating_duration_since(entry.last_used) < idle_eviction);
        previous - entries.len()
    }

    #[cfg(test)]
    fn contains(&self, key: &K) -> bool {
        self.entries
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .contains_key(key)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config(
        minimum_samples: u32,
        close_successes: u32,
        half_open_concurrency: u32,
    ) -> BreakerConfig {
        BreakerConfig::new(
            Duration::from_secs(10),
            10,
            minimum_samples,
            0.5,
            Duration::from_secs(10),
            Duration::from_secs(120),
            half_open_concurrency,
            close_successes,
        )
    }

    fn success(breaker: &Arc<CircuitBreaker>) {
        breaker.try_acquire().unwrap().succeed();
    }

    fn failure(breaker: &Arc<CircuitBreaker>) {
        breaker.try_acquire().unwrap().fail(FailureClass::Transport);
    }

    #[tokio::test(start_paused = true)]
    async fn rolling_window_expires_old_failures() {
        let breaker = CircuitBreaker::endpoint(config(4, 1, 1));
        failure(&breaker);
        failure(&breaker);
        tokio::time::advance(Duration::from_secs(10)).await;
        success(&breaker);
        success(&breaker);

        let snapshot = breaker.snapshot();
        assert_eq!(snapshot.state, BreakerState::Closed);
        assert_eq!(snapshot.samples, 2);
        assert_eq!(snapshot.failures, 0);
    }

    #[tokio::test(start_paused = true)]
    async fn open_half_open_and_closed_enforce_probe_concurrency_and_successes() {
        let breaker = CircuitBreaker::service(config(2, 2, 1));
        failure(&breaker);
        failure(&breaker);
        assert!(matches!(
            breaker.try_acquire(),
            Err(BreakerRejection::Open { retry_after })
                if retry_after == Duration::from_secs(10)
        ));

        tokio::time::advance(Duration::from_secs(10)).await;
        let first_probe = breaker.try_acquire().unwrap();
        assert_eq!(
            breaker.try_acquire().unwrap_err(),
            BreakerRejection::HalfOpenSaturated
        );
        first_probe.succeed();
        assert_eq!(breaker.snapshot().state, BreakerState::HalfOpen);

        breaker.try_acquire().unwrap().succeed();
        let snapshot = breaker.snapshot();
        assert_eq!(snapshot.state, BreakerState::Closed);
    }

    #[tokio::test(start_paused = true)]
    async fn transition_observer_receives_each_terminal_phase_outside_the_state_lock() {
        let phases = Arc::new(Mutex::new(Vec::new()));
        let breaker = CircuitBreaker::observed(config(1, 1, 1), {
            let phases = phases.clone();
            Arc::new(move |phase| {
                phases
                    .lock()
                    .unwrap_or_else(|error| error.into_inner())
                    .push(phase);
            })
        });

        failure(&breaker);
        tokio::time::advance(Duration::from_secs(10)).await;
        success(&breaker);

        assert_eq!(
            *phases.lock().unwrap_or_else(|error| error.into_inner()),
            [
                BreakerPhase::Open,
                BreakerPhase::HalfOpen,
                BreakerPhase::Closed,
            ]
        );
    }

    #[tokio::test(start_paused = true)]
    async fn ignored_half_open_outcomes_preserve_success_streak() {
        for ignored in [FailureClass::Cancelled, FailureClass::LocalRejection] {
            let breaker = CircuitBreaker::endpoint(config(1, 2, 1));
            failure(&breaker);
            tokio::time::advance(Duration::from_secs(10)).await;

            breaker.try_acquire().unwrap().succeed();
            breaker.try_acquire().unwrap().fail(ignored);
            breaker.try_acquire().unwrap().succeed();
            assert_eq!(breaker.snapshot().state, BreakerState::Closed);
        }
    }

    #[tokio::test(start_paused = true)]
    async fn unattempted_half_open_reservation_preserves_success_streak() {
        let breaker = CircuitBreaker::endpoint(config(1, 2, 1));
        failure(&breaker);
        tokio::time::advance(Duration::from_secs(10)).await;

        breaker.try_acquire().unwrap().succeed();
        breaker.try_acquire().unwrap().release_unattempted();
        breaker.try_acquire().unwrap().succeed();
        assert_eq!(breaker.snapshot().state, BreakerState::Closed);
    }

    #[tokio::test(start_paused = true)]
    async fn repeated_half_open_failure_doubles_open_time_with_two_minute_cap() {
        let breaker = CircuitBreaker::endpoint(BreakerConfig::new(
            Duration::from_secs(1),
            1,
            1,
            1.0,
            Duration::from_secs(10),
            Duration::from_secs(600),
            1,
            1,
        ));
        failure(&breaker);

        for expected in [20, 40, 80, 120, 120] {
            let current = breaker.snapshot().retry_after.unwrap();
            tokio::time::advance(current).await;
            failure(&breaker);
            assert_eq!(
                breaker.snapshot().retry_after,
                Some(Duration::from_secs(expected))
            );
        }
    }

    #[tokio::test(start_paused = true)]
    async fn endpoint_store_is_lru_bounded_and_evicts_ten_minute_idle_entries() {
        let store = EndpointBreakerStore::new(config(2, 1, 1), 2, DEFAULT_ENDPOINT_IDLE_EVICTION);
        store.get_or_insert("first");
        tokio::time::advance(Duration::from_secs(1)).await;
        store.get_or_insert("second");
        tokio::time::advance(Duration::from_secs(1)).await;
        store.get_or_insert("first");
        store.get_or_insert("third");

        assert_eq!(store.len(), 2);
        assert!(store.contains(&"first"));
        assert!(!store.contains(&"second"));
        assert!(store.contains(&"third"));

        tokio::time::advance(DEFAULT_ENDPOINT_IDLE_EVICTION).await;
        assert_eq!(store.evict_idle(), 2);
        assert!(store.is_empty());
    }

    #[tokio::test(start_paused = true)]
    async fn touching_an_idle_endpoint_replaces_instead_of_reviving_its_breaker() {
        let store = EndpointBreakerStore::new(config(2, 1, 1), 2, Duration::from_secs(10));
        let original = store.get_or_insert("endpoint");
        tokio::time::advance(Duration::from_secs(10)).await;

        let replacement = store.get_or_insert("endpoint");
        assert!(!Arc::ptr_eq(&original, &replacement));
        assert_eq!(store.len(), 1);
    }

    #[test]
    fn failure_classification_separates_health_and_local_outcomes() {
        assert!(FailureClass::Timeout.is_retryable());
        assert_eq!(
            FailureClass::Application.breaker_impact(),
            BreakerImpact::Success
        );
        assert_eq!(
            FailureClass::Cancelled.breaker_impact(),
            BreakerImpact::Ignore
        );
        assert!(!FailureClass::InvalidRequest.is_retryable());
        assert!(!FailureClass::RemoteFailure.is_retryable());
        assert_eq!(
            FailureClass::RemoteFailure.breaker_impact(),
            BreakerImpact::Failure
        );
    }

    #[test]
    fn breaker_samples_only_health_relevant_outcomes() {
        let breaker = CircuitBreaker::endpoint(config(20, 1, 1));
        for ignored in [
            FailureClass::Overloaded,
            FailureClass::InvalidRequest,
            FailureClass::Cancelled,
            FailureClass::LocalRejection,
        ] {
            breaker.try_acquire().unwrap().fail(ignored);
        }
        breaker
            .try_acquire()
            .unwrap()
            .fail(FailureClass::Application);
        breaker.try_acquire().unwrap().fail(FailureClass::Protocol);

        let snapshot = breaker.snapshot();
        assert_eq!(snapshot.state, BreakerState::Closed);
        assert_eq!((snapshot.samples, snapshot.failures), (2, 1));
    }
}
