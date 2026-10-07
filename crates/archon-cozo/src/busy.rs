//! An acquisition pause is retryable, never a completed or failed operation.
use std::fmt;

#[derive(Clone, Debug)]
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

pub(crate) fn lock_window_busy(context: &str, detail: String) -> anyhow::Error {
    StoreBusy {
        context: context.into(),
        attempts: 1,
        detail,
    }
    .into()
}

impl StoreBusy {
    /// Find the typed pause even inside another library's error wrapper.
    pub fn find<'a>(mut error: &'a (dyn std::error::Error + 'static)) -> Option<&'a Self> {
        loop {
            if let Some(busy) = error.downcast_ref::<Self>() {
                return Some(busy);
            }
            error = error.source()?;
        }
    }
}
