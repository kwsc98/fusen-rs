use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

use bytes::Bytes;

#[derive(Debug)]
pub(crate) struct ByteBudget {
    limit: usize,
    used: AtomicUsize,
}

impl ByteBudget {
    pub(crate) fn new(limit: usize) -> Arc<Self> {
        debug_assert!(limit > 0);
        Arc::new(Self {
            limit,
            used: AtomicUsize::new(0),
        })
    }

    pub(crate) fn try_reserve(self: &Arc<Self>, bytes: usize) -> Option<BytePermit> {
        self.try_add(bytes).then(|| BytePermit {
            budget: self.clone(),
            bytes: AtomicUsize::new(bytes),
        })
    }

    #[cfg(test)]
    pub(crate) fn used(&self) -> usize {
        self.used.load(Ordering::Acquire)
    }

    fn try_add(&self, bytes: usize) -> bool {
        let mut current = self.used.load(Ordering::Acquire);
        loop {
            let Some(next) = current.checked_add(bytes) else {
                return false;
            };
            if next > self.limit {
                return false;
            }
            match self.used.compare_exchange_weak(
                current,
                next,
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                Ok(_) => return true,
                Err(actual) => current = actual,
            }
        }
    }

    fn release(&self, bytes: usize) {
        let previous = self.used.fetch_sub(bytes, Ordering::AcqRel);
        debug_assert!(previous >= bytes);
    }
}

#[derive(Debug)]
pub(crate) struct BytePermit {
    budget: Arc<ByteBudget>,
    bytes: AtomicUsize,
}

impl BytePermit {
    pub(crate) fn grow(&self, bytes: usize) -> bool {
        if self.budget.try_add(bytes) {
            self.bytes.fetch_add(bytes, Ordering::AcqRel);
            true
        } else {
            false
        }
    }

    pub(crate) fn bytes(&self) -> usize {
        self.bytes.load(Ordering::Acquire)
    }

    pub(crate) fn belongs_to(&self, budget: &Arc<ByteBudget>) -> bool {
        Arc::ptr_eq(&self.budget, budget)
    }
}

impl Drop for BytePermit {
    fn drop(&mut self) {
        self.budget.release(*self.bytes.get_mut());
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum BudgetedWriteFailure {
    LimitExceeded,
    BudgetExhausted,
}

pub(crate) struct BudgetedWriter {
    bytes: Vec<u8>,
    limit: usize,
    permit: BytePermit,
    failure: Option<BudgetedWriteFailure>,
}

impl BudgetedWriter {
    pub(crate) fn new(
        limit: usize,
        budget: &Arc<ByteBudget>,
        wire_overhead: usize,
    ) -> Result<Self, BudgetedWriteFailure> {
        let permit = budget
            .try_reserve(wire_overhead)
            .ok_or(BudgetedWriteFailure::BudgetExhausted)?;
        Ok(Self {
            bytes: Vec::new(),
            limit,
            permit,
            failure: None,
        })
    }

    pub(crate) const fn failure(&self) -> Option<BudgetedWriteFailure> {
        self.failure
    }

    pub(crate) fn into_parts(self) -> (Bytes, Arc<BytePermit>) {
        (Bytes::from(self.bytes), Arc::new(self.permit))
    }
}

impl std::io::Write for BudgetedWriter {
    fn write(&mut self, buffer: &[u8]) -> std::io::Result<usize> {
        if self
            .bytes
            .len()
            .checked_add(buffer.len())
            .is_none_or(|length| length > self.limit)
        {
            self.failure = Some(BudgetedWriteFailure::LimitExceeded);
            return Err(std::io::Error::other("bounded writer limit exceeded"));
        }
        if !self.permit.grow(buffer.len()) {
            self.failure = Some(BudgetedWriteFailure::BudgetExhausted);
            return Err(std::io::Error::other("byte budget exhausted"));
        }
        self.bytes.extend_from_slice(buffer);
        Ok(buffer.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reservations_are_bounded_and_released_by_raii() {
        let budget = ByteBudget::new(8);
        let permit = budget.try_reserve(4).unwrap();
        assert!(permit.grow(4));
        assert!(!permit.grow(1));
        assert!(budget.try_reserve(1).is_none());
        assert_eq!(permit.bytes(), 8);
        drop(permit);
        assert_eq!(budget.used(), 0);
    }

    #[test]
    fn budgeted_writer_reserves_before_growth_and_releases_on_failure() {
        use std::io::Write;

        let budget = ByteBudget::new(4);
        let mut writer = BudgetedWriter::new(8, &budget, 1).unwrap();
        writer.write_all(b"abc").unwrap();
        assert_eq!(budget.used(), 4);
        assert!(writer.write_all(b"d").is_err());
        assert_eq!(
            writer.failure(),
            Some(BudgetedWriteFailure::BudgetExhausted)
        );
        assert_eq!(budget.used(), 4);
        drop(writer);
        assert_eq!(budget.used(), 0);
    }
}
