//! Bounded retry decisions, token budgets, and full-jitter backoff.

use super::FailureClass;
use rand::{Rng, RngExt};
use std::{
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::time::Instant;

const TOKEN_SCALE: u128 = 1_000_000_000;

/// Runtime-enforced maximum number of physical attempts per logical invocation.
pub(crate) const HARD_MAX_ATTEMPTS: u8 = 3;

/// Decision returned by a [`RetryPolicy`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum RetryDecision {
    /// Permit another physical attempt, subject to runtime limits and retry budget.
    Retry,
    /// Return the latest failure without another attempt.
    Stop,
}

/// Read-only input supplied to a [`RetryPolicy`] after a failed physical attempt.
///
/// The runtime enforces its hard attempt limit and token budget after the policy returns, so a
/// custom policy cannot bypass those process-safety limits.
#[derive(Clone, Copy, Debug)]
#[non_exhaustive]
pub struct RetryDecisionContext {
    completed_attempts: u8,
    max_attempts: u8,
    method_allows_retries: bool,
    failure: FailureClass,
    remaining: Duration,
}

impl RetryDecisionContext {
    pub(crate) fn new(
        completed_attempts: u8,
        configured_max_attempts: u8,
        method_allows_retries: bool,
        failure: FailureClass,
        remaining: Duration,
    ) -> Self {
        Self {
            completed_attempts: completed_attempts.max(1),
            max_attempts: hard_attempt_limit(configured_max_attempts),
            method_allows_retries,
            failure,
            remaining,
        }
    }

    /// Returns the number of physical attempts already completed, including the failed attempt.
    pub const fn completed_attempts(&self) -> u8 {
        self.completed_attempts
    }

    /// Returns the effective configured attempt limit after applying the runtime hard cap.
    pub const fn max_attempts(&self) -> u8 {
        self.max_attempts
    }

    /// Returns whether the method's standard HTTP mapping permits automatic replay.
    pub const fn method_allows_retries(&self) -> bool {
        self.method_allows_retries
    }

    /// Returns the stable failure classification for the completed attempt.
    pub const fn failure(&self) -> FailureClass {
        self.failure
    }

    /// Returns the logical invocation budget remaining before any retry delay.
    pub const fn remaining(&self) -> Duration {
        self.remaining
    }
}

/// Extension point for deciding whether a failed replayable invocation is eligible for retry.
///
/// Implementations should be fast and side-effect free. The runtime separately enforces method
/// HTTP replay eligibility, its non-retryable failure matrix, the absolute deadline, the hard
/// three-attempt cap, and the shared retry token budget.
pub trait RetryPolicy: Send + Sync + 'static {
    /// Evaluates one failed physical attempt.
    fn decide(&self, context: &RetryDecisionContext) -> RetryDecision;
}

impl<T> RetryPolicy for Arc<T>
where
    T: RetryPolicy + ?Sized,
{
    fn decide(&self, context: &RetryDecisionContext) -> RetryDecision {
        (**self).decide(context)
    }
}

/// Built-in conservative retry policy used when no extension is installed.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct StandardRetryPolicy;

impl RetryPolicy for StandardRetryPolicy {
    fn decide(&self, context: &RetryDecisionContext) -> RetryDecision {
        if context.method_allows_retries() && context.failure().is_retryable() {
            RetryDecision::Retry
        } else {
            RetryDecision::Stop
        }
    }
}

/// Per-service token bucket that bounds retry amplification.
///
/// Tokens are replenished continuously using Tokio's monotonic clock. Fixed-point accounting
/// preserves fractional tokens across calls without floating-point drift.
#[derive(Debug)]
pub(crate) struct RetryBudget {
    capacity: u32,
    refill_per_second: u32,
    state: Mutex<BudgetState>,
}

#[derive(Debug)]
struct BudgetState {
    scaled_tokens: u128,
    last_refill: Instant,
}

impl RetryBudget {
    pub(crate) fn new(capacity: u32, refill_per_second: u32) -> Self {
        debug_assert!(capacity > 0);
        debug_assert!(refill_per_second > 0);
        Self {
            capacity,
            refill_per_second,
            state: Mutex::new(BudgetState {
                scaled_tokens: u128::from(capacity) * TOKEN_SCALE,
                last_refill: Instant::now(),
            }),
        }
    }

    pub(crate) fn try_acquire(&self) -> bool {
        let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        self.refill(&mut state, Instant::now());
        if state.scaled_tokens < TOKEN_SCALE {
            return false;
        }
        state.scaled_tokens -= TOKEN_SCALE;
        true
    }

    #[cfg(test)]
    pub(crate) fn available(&self) -> u32 {
        let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        self.refill(&mut state, Instant::now());
        (state.scaled_tokens / TOKEN_SCALE) as u32
    }

    fn refill(&self, state: &mut BudgetState, now: Instant) {
        let elapsed = now.saturating_duration_since(state.last_refill);
        let replenished = elapsed
            .as_nanos()
            .saturating_mul(u128::from(self.refill_per_second));
        let maximum = u128::from(self.capacity) * TOKEN_SCALE;
        state.scaled_tokens = state.scaled_tokens.saturating_add(replenished).min(maximum);
        state.last_refill = now;
    }
}

/// Applies non-overridable limits and consumes one retry token after a policy opts in.
pub(crate) fn decide_with_guards(
    policy: &dyn RetryPolicy,
    context: &RetryDecisionContext,
    budget: &RetryBudget,
) -> RetryDecision {
    if context.remaining().is_zero()
        || !has_next_attempt(context.completed_attempts(), context.max_attempts())
        || !context.method_allows_retries()
        || !context.failure().is_retryable()
        || policy.decide(context) != RetryDecision::Retry
        || !budget.try_acquire()
    {
        RetryDecision::Stop
    } else {
        RetryDecision::Retry
    }
}

/// Returns a full-jitter delay in `0..=min(cap, base * 2^(retry - 1))`.
pub(crate) fn full_jitter_backoff<R>(
    base: Duration,
    cap: Duration,
    retry_ordinal: u8,
    rng: &mut R,
) -> Duration
where
    R: Rng + ?Sized,
{
    let ceiling = exponential_ceiling(base, cap, retry_ordinal);
    let nanos = rng.random_range(0..=ceiling.as_nanos());
    duration_from_nanos(nanos)
}

pub(crate) const fn hard_attempt_limit(configured: u8) -> u8 {
    if configured == 0 {
        1
    } else if configured > HARD_MAX_ATTEMPTS {
        HARD_MAX_ATTEMPTS
    } else {
        configured
    }
}

pub(crate) const fn has_next_attempt(completed_attempts: u8, configured: u8) -> bool {
    completed_attempts < hard_attempt_limit(configured)
}

fn exponential_ceiling(base: Duration, cap: Duration, retry_ordinal: u8) -> Duration {
    let shift = u32::from(retry_ordinal.saturating_sub(1)).min(31);
    base.checked_mul(1_u32 << shift).unwrap_or(cap).min(cap)
}

fn duration_from_nanos(nanos: u128) -> Duration {
    const NANOS_PER_SECOND: u128 = 1_000_000_000;
    let seconds = nanos / NANOS_PER_SECOND;
    let subsecond_nanos = (nanos % NANOS_PER_SECOND) as u32;
    Duration::new(seconds as u64, subsecond_nanos)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand::{SeedableRng, rngs::StdRng};

    #[tokio::test(start_paused = true)]
    async fn retry_budget_preserves_fractional_refills_and_caps_capacity() {
        let budget = RetryBudget::new(2, 2);
        assert!(budget.try_acquire());
        assert!(budget.try_acquire());
        assert!(!budget.try_acquire());

        tokio::time::advance(Duration::from_millis(250)).await;
        assert!(!budget.try_acquire());
        tokio::time::advance(Duration::from_millis(250)).await;
        assert!(budget.try_acquire());
        assert!(!budget.try_acquire());

        tokio::time::advance(Duration::from_secs(10)).await;
        assert_eq!(budget.available(), 2);
    }

    #[test]
    fn hard_attempt_cap_cannot_be_overridden_by_configuration_or_policy() {
        assert_eq!(hard_attempt_limit(0), 1);
        assert_eq!(hard_attempt_limit(2), 2);
        assert_eq!(hard_attempt_limit(20), HARD_MAX_ATTEMPTS);
        assert!(has_next_attempt(1, 3));
        assert!(has_next_attempt(2, 20));
        assert!(!has_next_attempt(3, 20));

        struct AlwaysRetry;
        impl RetryPolicy for AlwaysRetry {
            fn decide(&self, _: &RetryDecisionContext) -> RetryDecision {
                RetryDecision::Retry
            }
        }

        let budget = RetryBudget::new(10, 1);
        let context = RetryDecisionContext::new(
            3,
            100,
            true,
            FailureClass::Transport,
            Duration::from_secs(1),
        );
        assert_eq!(
            decide_with_guards(&AlwaysRetry, &context, &budget),
            RetryDecision::Stop
        );
        assert_eq!(budget.available(), 10);
    }

    #[test]
    fn standard_policy_only_retries_replayable_transient_failures() {
        let budget = RetryBudget::new(2, 1);
        let context = RetryDecisionContext::new(
            1,
            3,
            true,
            FailureClass::Unavailable,
            Duration::from_secs(1),
        );
        assert_eq!(
            decide_with_guards(&StandardRetryPolicy, &context, &budget),
            RetryDecision::Retry
        );

        let unsafe_context = RetryDecisionContext::new(
            1,
            3,
            false,
            FailureClass::Unavailable,
            Duration::from_secs(1),
        );
        assert_eq!(
            decide_with_guards(&StandardRetryPolicy, &unsafe_context, &budget),
            RetryDecision::Stop
        );
    }

    #[test]
    fn retry_gate_does_not_spend_tokens_before_all_non_budget_guards_pass() {
        struct AlwaysRetry;
        impl RetryPolicy for AlwaysRetry {
            fn decide(&self, _: &RetryDecisionContext) -> RetryDecision {
                RetryDecision::Retry
            }
        }

        let budget = RetryBudget::new(1, 1);
        let elapsed =
            RetryDecisionContext::new(1, 3, true, FailureClass::Transport, Duration::ZERO);
        assert_eq!(
            decide_with_guards(&AlwaysRetry, &elapsed, &budget),
            RetryDecision::Stop
        );
        assert_eq!(budget.available(), 1);

        let unsafe_method =
            RetryDecisionContext::new(1, 3, false, FailureClass::Transport, Duration::from_secs(1));
        assert_eq!(
            decide_with_guards(&AlwaysRetry, &unsafe_method, &budget),
            RetryDecision::Stop
        );
        assert_eq!(budget.available(), 1);

        let protocol_failure =
            RetryDecisionContext::new(1, 3, true, FailureClass::Protocol, Duration::from_secs(1));
        assert_eq!(
            decide_with_guards(&AlwaysRetry, &protocol_failure, &budget),
            RetryDecision::Stop
        );
        assert_eq!(budget.available(), 1);

        let eligible =
            RetryDecisionContext::new(1, 3, true, FailureClass::Transport, Duration::from_secs(1));
        assert_eq!(
            decide_with_guards(&AlwaysRetry, &eligible, &budget),
            RetryDecision::Retry
        );
        assert_eq!(budget.available(), 0);
        assert_eq!(
            decide_with_guards(&AlwaysRetry, &eligible, &budget),
            RetryDecision::Stop
        );
    }

    #[test]
    fn full_jitter_is_seeded_and_never_exceeds_exponential_cap() {
        let base = Duration::from_millis(10);
        let cap = Duration::from_millis(25);
        let mut first = StdRng::seed_from_u64(7);
        let mut second = StdRng::seed_from_u64(7);
        let expected_ceilings = [
            Duration::from_millis(10),
            Duration::from_millis(20),
            Duration::from_millis(25),
            Duration::from_millis(25),
        ];

        for (index, ceiling) in expected_ceilings.into_iter().enumerate() {
            let ordinal = (index + 1) as u8;
            let left = full_jitter_backoff(base, cap, ordinal, &mut first);
            let right = full_jitter_backoff(base, cap, ordinal, &mut second);
            assert_eq!(left, right);
            assert!(left <= ceiling);
        }
    }
}
