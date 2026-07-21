//! Persist explicit provider fallback decisions.

use super::learning_store;
use archon_llm::runtime::{
    ProviderRuntimeEvent, ProviderRuntimeEventType, ProviderRuntimeSeverity,
};

pub(crate) async fn record_provider_fallback_selected(
    provider_id: &str,
    from_runtime_mode: &str,
    to_runtime_mode: &str,
    reason_code: &str,
    metadata: serde_json::Value,
) {
    persist_fallback_decision(FallbackDecision {
        provider_id: provider_id.to_string(),
        runtime_mode: to_runtime_mode.to_string(),
        fallback_from: from_runtime_mode.to_string(),
        fallback_to: to_runtime_mode.to_string(),
        event_type: ProviderRuntimeEventType::FallbackSelected,
        severity: ProviderRuntimeSeverity::Warn,
        reason_code: reason_code.to_string(),
        metadata,
    })
    .await;
}

pub(crate) async fn record_provider_fallback_denied(
    provider_id: &str,
    from_runtime_mode: &str,
    to_runtime_mode: &str,
    reason_code: &str,
    metadata: serde_json::Value,
) {
    persist_fallback_decision(FallbackDecision {
        provider_id: provider_id.to_string(),
        runtime_mode: to_runtime_mode.to_string(),
        fallback_from: from_runtime_mode.to_string(),
        fallback_to: to_runtime_mode.to_string(),
        event_type: ProviderRuntimeEventType::FallbackDenied,
        severity: ProviderRuntimeSeverity::Error,
        reason_code: reason_code.to_string(),
        metadata,
    })
    .await;
}

pub(crate) async fn record_provider_construction_fallback_denied(
    requested_provider: &str,
    target_provider: &str,
    reason_code: &str,
    metadata: serde_json::Value,
) {
    persist_fallback_decision(FallbackDecision {
        provider_id: target_provider.to_string(),
        runtime_mode: "direct".to_string(),
        fallback_from: requested_provider.to_string(),
        fallback_to: target_provider.to_string(),
        event_type: ProviderRuntimeEventType::FallbackDenied,
        severity: ProviderRuntimeSeverity::Error,
        reason_code: reason_code.to_string(),
        metadata,
    })
    .await;
}

struct FallbackDecision {
    provider_id: String,
    runtime_mode: String,
    fallback_from: String,
    fallback_to: String,
    event_type: ProviderRuntimeEventType,
    severity: ProviderRuntimeSeverity,
    reason_code: String,
    metadata: serde_json::Value,
}

async fn persist_fallback_decision(decision: FallbackDecision) {
    run_fallback_persistence_async_with(move || record_provider_fallback_decision(decision)).await;
}

async fn run_fallback_persistence_async_with(persist: impl FnOnce() + Send + 'static) -> bool {
    match archon_tui::observability::spawn_blocking_named(
        "provider-fallback-persistence-write",
        persist,
    )
    .await
    {
        Ok(()) => true,
        Err(error) => {
            tracing::warn!(%error, "provider fallback persistence task failed");
            false
        }
    }
}

fn record_provider_fallback_decision(decision: FallbackDecision) {
    let db = match learning_store::acquire_default() {
        Ok(db) => db,
        Err(error) => {
            tracing::warn!(
                %error,
                provider = decision.provider_id,
                event_action = ?decision.event_type,
                "provider fallback event persistence unavailable"
            );
            return;
        }
    };
    let event = ProviderRuntimeEvent::new(
        &decision.provider_id,
        &decision.runtime_mode,
        decision.event_type,
        decision.severity,
    )
    .with_reason(decision.reason_code)
    .with_fallback(decision.fallback_from, decision.fallback_to)
    .with_redacted_json(decision.metadata);
    let record = crate::runtime::provider_event_record::provider_event_record(event);
    if let Err(error) = archon_learning::runtime_events::insert_provider_runtime_event(&db, &record)
    {
        tracing::warn!(%error, provider_id = %decision.provider_id, "provider fallback event persistence failed");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test(flavor = "current_thread")]
    async fn async_fallback_persistence_does_not_block_runtime_worker() {
        use std::sync::mpsc;
        use std::time::Duration;

        let (write_started_tx, write_started_rx) = mpsc::channel();
        let (release_write_tx, release_write_rx) = mpsc::channel();
        let (progress_tx, progress_rx) = mpsc::channel();
        let (result_tx, result_rx) = mpsc::channel();

        let coordinator = std::thread::spawn(move || {
            write_started_rx
                .recv_timeout(Duration::from_secs(2))
                .expect("fallback persistence entered");
            let progressed = progress_rx.recv_timeout(Duration::from_millis(250)).is_ok();
            release_write_tx
                .send(())
                .expect("release fallback persistence");
            result_tx.send(progressed).expect("report runtime progress");
        });

        let persistence = run_fallback_persistence_async_with(move || {
            write_started_tx
                .send(())
                .expect("announce fallback persistence");
            release_write_rx
                .recv()
                .expect("release fallback persistence");
        });
        let progress = async move {
            progress_tx.send(()).expect("report runtime progress");
        };
        let (persisted, ()) = tokio::join!(persistence, progress);

        coordinator.join().expect("coordinator joins");
        assert!(persisted, "fallback persistence task must complete");
        assert!(
            result_rx.recv().expect("runtime progress result"),
            "another Tokio task must progress while fallback persistence blocks"
        );
    }

    #[test]
    fn construction_fallback_denial_event_uses_provider_fallback_fields() {
        let event = ProviderRuntimeEvent::new(
            "anthropic",
            "direct",
            ProviderRuntimeEventType::FallbackDenied,
            ProviderRuntimeSeverity::Error,
        )
        .with_reason("anthropic_fallback_auth_unavailable")
        .with_fallback("openai", "anthropic");
        let record = crate::runtime::provider_event_record::provider_event_record(event);

        assert_eq!(record.provider_id, "anthropic");
        assert_eq!(record.event_type, "fallback_denied");
        assert_eq!(record.fallback_from.as_deref(), Some("openai"));
        assert_eq!(record.fallback_to.as_deref(), Some("anthropic"));
        assert_eq!(
            record.reason_code.as_deref(),
            Some("anthropic_fallback_auth_unavailable")
        );
    }
}
