//! An exhausted busy attempt window is a retryable outcome, never a completed operation.
use std::fmt;

#[derive(Debug)]
pub struct StoreBusy {
    pub context: String,
    pub attempts: usize,
    pub detail: String,
}

impl fmt::Display for StoreBusy {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}: retryable store busy after {} attempts; operation not completed: {}",
            self.context, self.attempts, self.detail
        )
    }
}
impl std::error::Error for StoreBusy {}

pub(crate) fn guarded_error(context: &str, attempts: usize, detail: String) -> anyhow::Error {
    if crate::is_store_contention(&detail) {
        StoreBusy {
            context: context.into(),
            attempts,
            detail,
        }
        .into()
    } else {
        anyhow::anyhow!("{context}: {detail}")
    }
}

pub(crate) fn lock_window_busy(context: &str, detail: String) -> anyhow::Error {
    StoreBusy {
        context: context.into(),
        attempts: 1,
        detail,
    }
    .into()
}
