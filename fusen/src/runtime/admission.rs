use std::sync::{
    Arc,
    atomic::{AtomicU8, AtomicUsize, Ordering},
};
use tokio::sync::{Notify, OwnedSemaphorePermit, Semaphore};

const RUNNING: u8 = 0;
const DRAINING: u8 = 1;
const CLOSED: u8 = 2;

pub(crate) struct AdmissionGate {
    state: AtomicU8,
    permits: Arc<Semaphore>,
    active: AtomicUsize,
    changed: Notify,
}

impl AdmissionGate {
    pub(crate) fn new(limit: usize) -> Arc<Self> {
        debug_assert!(limit > 0);
        Arc::new(Self {
            state: AtomicU8::new(RUNNING),
            permits: Arc::new(Semaphore::new(limit)),
            active: AtomicUsize::new(0),
            changed: Notify::new(),
        })
    }

    pub(crate) fn try_enter(self: &Arc<Self>) -> Result<AdmissionGuard, AdmissionError> {
        if self.state.load(Ordering::Acquire) != RUNNING {
            return Err(AdmissionError::Draining);
        }
        let permit = self.permits.clone().try_acquire_owned().map_err(|_| {
            if self.state.load(Ordering::Acquire) == RUNNING {
                AdmissionError::Overloaded
            } else {
                AdmissionError::Draining
            }
        })?;
        self.active.fetch_add(1, Ordering::AcqRel);
        if self.state.load(Ordering::Acquire) != RUNNING {
            self.release_one();
            drop(permit);
            return Err(AdmissionError::Draining);
        }
        Ok(AdmissionGuard {
            gate: self.clone(),
            _permit: permit,
        })
    }

    pub(crate) async fn enter(self: &Arc<Self>) -> Result<AdmissionGuard, AdmissionError> {
        if self.state.load(Ordering::Acquire) != RUNNING {
            return Err(AdmissionError::Draining);
        }
        let permit = self
            .permits
            .clone()
            .acquire_owned()
            .await
            .map_err(|_| AdmissionError::Draining)?;
        self.active.fetch_add(1, Ordering::AcqRel);
        if self.state.load(Ordering::Acquire) != RUNNING {
            self.release_one();
            drop(permit);
            return Err(AdmissionError::Draining);
        }
        Ok(AdmissionGuard {
            gate: self.clone(),
            _permit: permit,
        })
    }

    pub(crate) fn begin_draining(&self) -> bool {
        let changed = self
            .state
            .compare_exchange(RUNNING, DRAINING, Ordering::AcqRel, Ordering::Acquire)
            .is_ok();
        if changed {
            self.permits.close();
            self.changed.notify_waiters();
        }
        changed
    }

    pub(crate) fn close(&self) {
        self.state.store(CLOSED, Ordering::Release);
        self.permits.close();
        self.changed.notify_waiters();
    }

    pub(crate) fn active(&self) -> usize {
        self.active.load(Ordering::Acquire)
    }

    pub(crate) async fn drained(&self) {
        loop {
            let notified = self.changed.notified();
            if self.active() == 0 {
                return;
            }
            notified.await;
        }
    }

    fn release_one(&self) {
        let previous = self.active.fetch_sub(1, Ordering::AcqRel);
        debug_assert!(previous > 0);
        if previous == 1 {
            self.changed.notify_waiters();
        }
    }
}

pub(crate) struct AdmissionGuard {
    gate: Arc<AdmissionGate>,
    _permit: OwnedSemaphorePermit,
}

impl Drop for AdmissionGuard {
    fn drop(&mut self) {
        self.gate.release_one();
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum AdmissionError {
    Overloaded,
    Draining,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn admission_is_fail_fast_and_drains_linearly() {
        let gate = AdmissionGate::new(1);
        let guard = gate.try_enter().unwrap();
        assert!(matches!(gate.try_enter(), Err(AdmissionError::Overloaded)));

        let queued = tokio::spawn({
            let gate = gate.clone();
            async move { gate.enter().await }
        });
        tokio::task::yield_now().await;
        assert!(gate.begin_draining());
        assert!(matches!(gate.try_enter(), Err(AdmissionError::Draining)));
        assert!(matches!(
            queued.await.unwrap(),
            Err(AdmissionError::Draining)
        ));
        drop(guard);
        gate.drained().await;
        assert_eq!(gate.active(), 0);
    }
}
