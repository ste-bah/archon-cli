use std::collections::VecDeque;
use std::sync::Mutex;

pub const ASYNC_HOOK_DIAGNOSTIC_CAPACITY: usize = 128;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct AsyncHookDiagnostic {
    pub event: String,
    pub source: Option<String>,
    pub outcome: String,
    pub message: String,
}

impl AsyncHookDiagnostic {
    pub(crate) fn new(event: &str, source: Option<String>, outcome: &str, message: String) -> Self {
        Self {
            event: event.to_owned(),
            source,
            outcome: outcome.to_owned(),
            message,
        }
    }

    #[cfg(test)]
    fn test(event: &str) -> Self {
        Self::new(event, None, "success", String::new())
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct AsyncHookDiagnosticBatch {
    pub diagnostics: Vec<AsyncHookDiagnostic>,
    pub dropped: usize,
}

pub(crate) struct AsyncHookDiagnosticStore {
    capacity: usize,
    state: Mutex<StoreState>,
    observer: Mutex<Option<std::sync::Arc<dyn Fn(AsyncHookDiagnostic) + Send + Sync>>>,
}

#[derive(Default)]
struct StoreState {
    diagnostics: VecDeque<AsyncHookDiagnostic>,
    dropped: usize,
    closed: bool,
}

impl AsyncHookDiagnosticStore {
    pub(crate) fn new() -> Self {
        Self {
            capacity: ASYNC_HOOK_DIAGNOSTIC_CAPACITY,
            state: Mutex::new(StoreState::default()),
            observer: Mutex::new(None),
        }
    }

    #[cfg(test)]
    fn with_capacity(capacity: usize) -> Self {
        Self {
            capacity,
            state: Mutex::new(StoreState::default()),
            observer: Mutex::new(None),
        }
    }

    pub(crate) fn set_observer(
        &self,
        observer: Option<std::sync::Arc<dyn Fn(AsyncHookDiagnostic) + Send + Sync>>,
    ) {
        *self.observer.lock().unwrap_or_else(|p| p.into_inner()) = observer.clone();
        if let Some(observer) = observer {
            let diagnostics = self
                .state
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .diagnostics
                .iter()
                .cloned()
                .collect::<Vec<_>>();
            for diagnostic in diagnostics {
                observer(diagnostic);
            }
        }
    }

    pub(crate) fn push(&self, diagnostic: AsyncHookDiagnostic) {
        let mut state = self.state.lock().unwrap_or_else(|p| p.into_inner());
        if state.closed {
            state.dropped += 1;
            tracing::warn!(
                event = %diagnostic.event,
                dropped_count = state.dropped,
                "dropped async hook diagnostic after session result emission"
            );
            return;
        }
        if self.capacity == 0 {
            state.dropped += 1;
            return;
        }
        if state.diagnostics.len() == self.capacity {
            state.diagnostics.pop_front();
            state.dropped += 1;
        }
        state.diagnostics.push_back(diagnostic.clone());
        drop(state);
        if let Some(observer) = self
            .observer
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .clone()
        {
            observer(diagnostic);
        }
    }

    pub(crate) fn drain(&self) -> AsyncHookDiagnosticBatch {
        let mut state = self.state.lock().unwrap_or_else(|p| p.into_inner());
        AsyncHookDiagnosticBatch {
            diagnostics: state.diagnostics.drain(..).collect(),
            dropped: std::mem::take(&mut state.dropped),
        }
    }

    pub(crate) fn close_and_drain(&self) -> AsyncHookDiagnosticBatch {
        let mut state = self.state.lock().unwrap_or_else(|p| p.into_inner());
        state.closed = true;
        let batch = AsyncHookDiagnosticBatch {
            diagnostics: state.diagnostics.drain(..).collect(),
            dropped: std::mem::take(&mut state.dropped),
        };
        drop(state);
        *self.observer.lock().unwrap_or_else(|p| p.into_inner()) = None;
        batch
    }
}

#[cfg(test)]
#[path = "async_diagnostics_tests.rs"]
mod tests;
