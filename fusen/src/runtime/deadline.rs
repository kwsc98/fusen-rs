use std::{future::Future, time::Duration};
use tokio::time::Instant;

#[derive(Clone, Copy, Debug)]
pub(crate) struct Deadline(Instant);

impl Deadline {
    pub(crate) fn after(timeout: Duration) -> Self {
        Self(Instant::now() + timeout)
    }

    pub(crate) fn remaining(self) -> Duration {
        self.0.saturating_duration_since(Instant::now())
    }

    pub(crate) fn is_elapsed(self) -> bool {
        self.0 <= Instant::now()
    }

    pub(crate) fn min(self, other: Self) -> Self {
        Self(self.0.min(other.0))
    }

    pub(crate) async fn run<F>(self, future: F) -> Result<F::Output, DeadlineElapsed>
    where
        F: Future,
    {
        tokio::time::timeout_at(self.0, future)
            .await
            .map_err(|_| DeadlineElapsed)
    }
}

#[derive(Clone, Copy, Debug, thiserror::Error)]
#[error("deadline elapsed")]
pub(crate) struct DeadlineElapsed;

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test(start_paused = true)]
    async fn one_absolute_deadline_bounds_all_work() {
        let deadline = Deadline::after(Duration::from_secs(5));
        tokio::time::advance(Duration::from_secs(3)).await;
        assert_eq!(deadline.remaining(), Duration::from_secs(2));
        let result = deadline
            .run(tokio::time::sleep(Duration::from_secs(3)))
            .await;
        assert!(result.is_err());
    }
}
