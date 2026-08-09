//! Persistence side of provider observation: the recorder that writes runtime
//! events, logical call usage and limit windows, and the blocking-pool helper
//! they all go through.
//!
//! Split out of `provider_observer.rs`, which crossed the 500-line ceiling when
//! the wrapper gained alias forwarding. The seam was already there: nothing here
//! knows about `LlmProvider`, and the wrapper that remains never writes a row.
//!
//! Declared as a `#[path]` child of `provider_observer`, not a sibling under
//! `runtime`, because `provider_observer_stream` calls `persist` and
//! `record_sync`. Those are reachable from a sibling child only while their
//! visibility is `pub(super)` — scoped to the `provider_observer` subtree — which
//! is why they carry it rather than being private.

use archon_llm::provider::LlmError;
use archon_llm::runtime::ProviderRuntimeEvent;
use archon_llm::types::Usage;
use cozo::DbInstance;
use std::sync::Arc;

use super::usage::{ObservedRequest, logical_call_usage};
use crate::runtime::learning_store;
use crate::runtime::provider_event_record::provider_event_record;
use crate::runtime::provider_limit_windows;

#[derive(Clone)]
pub(crate) struct ProviderRuntimeEventRecorder {
    /// `pub(super)` rather than private: `provider_observer_stream` reads the
    /// handle directly to write stream-scoped rows, and it is a sibling child of
    /// `provider_observer` rather than a descendant of this module.
    pub(super) db: Option<Arc<DbInstance>>,
}

impl ProviderRuntimeEventRecorder {
    pub(crate) async fn default_learning_store() -> Self {
        let acquired = run_provider_persistence_async_with(
            "acquire provider runtime event store",
            learning_store::acquire_default,
        )
        .await;
        match acquired {
            Some(Ok(db)) => Self { db: Some(db) },
            Some(Err(error)) => {
                tracing::warn!(%error, "provider runtime event store unavailable");
                Self { db: None }
            }
            None => Self { db: None },
        }
    }

    #[cfg(test)]
    pub(super) fn with_db(db: Arc<DbInstance>) -> Self {
        Self { db: Some(db) }
    }

    pub(super) fn record_sync(&self, event: ProviderRuntimeEvent) -> Option<String> {
        let event_id = event.event_id.clone();
        let Some(db) = &self.db else {
            return None;
        };
        let record = provider_event_record(event);
        if let Err(error) =
            archon_learning::runtime_events::insert_provider_runtime_event(db, &record)
        {
            tracing::warn!(
                %error,
                provider = %record.provider_id,
                event_type = %record.event_type,
                "provider runtime event persistence failed"
            );
            return None;
        }
        Some(event_id)
    }

    pub(super) async fn persist<T>(
        &self,
        context: &'static str,
        persist: impl FnOnce(Self) -> T + Send + 'static,
    ) -> Option<T>
    where
        T: Send + 'static,
    {
        let recorder = self.clone();
        run_provider_persistence_async_with(context, move || persist(recorder)).await
    }

    pub(super) fn record_call_usage_sync(
        &self,
        request_id: &str,
        request: &ObservedRequest,
        usage: Option<&Usage>,
        status: &str,
    ) {
        let Some(db) = &self.db else {
            return;
        };
        let record = logical_call_usage(request_id, request, usage, status);
        if let Err(error) = archon_learning::llm_call_usage::insert_llm_call_usage(db, &record) {
            tracing::warn!(%error, request_id, "logical call usage persistence failed");
        }
    }

    pub(super) async fn record(&self, event: ProviderRuntimeEvent) -> Option<String> {
        self.persist("record provider runtime event", move |recorder| {
            recorder.record_sync(event)
        })
        .await
        .flatten()
    }

    pub(super) fn record_limit_window_sync(
        &self,
        provider_id: &str,
        model_id: Option<&str>,
        error: &LlmError,
    ) {
        provider_limit_windows::record_limit_window(self.db.as_ref(), provider_id, model_id, error);
    }
}

pub(super) async fn run_provider_persistence_async_with<T>(
    context: &'static str,
    persist: impl FnOnce() -> T + Send + 'static,
) -> Option<T>
where
    T: Send + 'static,
{
    match archon_tui::observability::spawn_blocking_named("provider-runtime-persistence", persist)
        .await
    {
        Ok(value) => Some(value),
        Err(error) => {
            tracing::warn!(%error, %context, "provider runtime persistence task failed");
            None
        }
    }
}
