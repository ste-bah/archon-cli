//! Forward provider stream events while recording terminal runtime outcomes.

use archon_llm::runtime::{ProviderRuntimeEventType, ProviderRuntimeSeverity};
use archon_llm::streaming::StreamEvent;
use archon_llm::types::Usage;
use archon_llm::usage::UsageAccumulator;
use tokio::sync::mpsc::Receiver;

use super::{ObservedRequest, ProviderRuntimeEventRecorder, base_event};

pub(super) fn forward_stream(
    mut inner_rx: Receiver<StreamEvent>,
    recorder: ProviderRuntimeEventRecorder,
    provider_id: String,
    runtime_mode: String,
    profile_id: Option<String>,
    observed: ObservedRequest,
    request_id: String,
) -> Receiver<StreamEvent> {
    let (tx, rx) = tokio::sync::mpsc::channel(64);
    tokio::spawn(async move {
        let mut completed = false;
        let mut terminal_status = None;
        let mut usage = UsageAccumulator::default();
        loop {
            let event = tokio::select! {
                _ = tx.closed() => {
                    while let Ok(event) = inner_rx.try_recv() {
                        usage.record_event(&event);
                    }
                    break;
                }
                event = inner_rx.recv() => event,
            };
            let Some(event) = event else {
                break;
            };
            usage.record_event(&event);
            match &event {
                StreamEvent::Error {
                    error_type,
                    message: _,
                } => {
                    terminal_status.get_or_insert("failed");
                    record_stream_error(
                        &recorder,
                        &provider_id,
                        &runtime_mode,
                        profile_id.as_deref(),
                        &observed,
                        &request_id,
                        error_type,
                    )
                    .await;
                }
                StreamEvent::MessageStop => {
                    if terminal_status.is_none() {
                        completed = true;
                        terminal_status = Some("succeeded");
                        record_stream_success(
                            &recorder,
                            &provider_id,
                            &runtime_mode,
                            profile_id.as_deref(),
                            &observed,
                            &request_id,
                        )
                        .await;
                    }
                }
                _ => {}
            }
            if tx.send(event).await.is_err() {
                break;
            }
        }
        let status = terminal_status.unwrap_or_else(|| {
            if tx.is_closed() {
                "consumer_closed"
            } else {
                "closed_without_stop"
            }
        });
        if !completed && status == "closed_without_stop" {
            record_stream_closed_without_stop(
                &recorder,
                &provider_id,
                &runtime_mode,
                profile_id.as_deref(),
                &observed,
                &request_id,
            )
            .await;
        }
        recorder
            .persist("record logical stream call usage", move |recorder| {
                recorder.record_call_usage_sync(
                    &request_id,
                    &observed,
                    Some(&usage_into_usage(&usage)),
                    status,
                );
            })
            .await;
    });
    rx
}

fn usage_into_usage(usage: &UsageAccumulator) -> Usage {
    Usage {
        input_tokens: usage.billable_input_tokens,
        output_tokens: usage.output_tokens,
        cache_creation_input_tokens: usage.cache_creation_input_tokens,
        cache_read_input_tokens: usage.cache_read_input_tokens,
        input_tokens_available: usage.input_tokens_available,
        output_tokens_available: usage.output_tokens_available,
        cache_creation_input_tokens_available: usage.cache_creation_input_tokens_available,
        cache_read_input_tokens_available: usage.cache_read_input_tokens_available,
    }
}

async fn record_stream_error(
    recorder: &ProviderRuntimeEventRecorder,
    provider_id: &str,
    runtime_mode: &str,
    profile_id: Option<&str>,
    observed: &ObservedRequest,
    request_id: &str,
    error_type: &str,
) {
    let event = base_event(
        provider_id,
        runtime_mode,
        ProviderRuntimeEventType::RequestFailed,
        ProviderRuntimeSeverity::Warn,
    )
    .with_request_id(request_id.to_string())
    .with_model(observed.model.clone())
    .with_reason(error_type.to_string())
    .with_message("provider stream emitted an error event")
    .with_redacted_json(serde_json::json!({
        "request_origin": observed.origin.as_deref(),
        "stream_error_type": error_type,
    }));
    let provider_id = provider_id.to_string();
    let runtime_mode = runtime_mode.to_string();
    let profile_id = profile_id.map(ToOwned::to_owned);
    let observed = observed.clone();
    let request_id = request_id.to_string();
    let error_type = error_type.to_string();
    recorder
        .persist("record provider stream error", move |recorder| {
            if let Some(event_id) = recorder.record_sync(event) {
                crate::runtime::provider_incident_ledger::record_provider_incident(
                    crate::runtime::provider_incident_ledger::ProviderIncidentLedgerInput {
                        db: recorder.db.as_ref(),
                        agent_type: observed.agent_type.as_deref(),
                        agent_version: observed.agent_version.as_deref(),
                        run_id: observed.run_id.as_deref(),
                        model_id: &observed.model,
                        provider_id: &provider_id,
                        provider_event_id: &event_id,
                        reason_code: &error_type,
                    },
                );
            }
            crate::runtime::provider_profile_updates::mark_failure_reason(
                recorder.db.as_ref(),
                &provider_id,
                &runtime_mode,
                profile_id.as_deref(),
                Some(&observed.model),
                Some(&request_id),
                &error_type,
            );
        })
        .await;
}

async fn record_stream_success(
    recorder: &ProviderRuntimeEventRecorder,
    provider_id: &str,
    runtime_mode: &str,
    profile_id: Option<&str>,
    observed: &ObservedRequest,
    request_id: &str,
) {
    let event = base_event(
        provider_id,
        runtime_mode,
        ProviderRuntimeEventType::RequestSucceeded,
        ProviderRuntimeSeverity::Info,
    )
    .with_request_id(request_id.to_string())
    .with_model(observed.model.clone())
    .with_reason("stream_completed")
    .with_redacted_json(serde_json::json!({
        "request_origin": observed.origin.as_deref(),
    }));
    let provider_id = provider_id.to_string();
    let runtime_mode = runtime_mode.to_string();
    let profile_id = profile_id.map(ToOwned::to_owned);
    let model = observed.model.clone();
    let request_id = request_id.to_string();
    recorder
        .persist("record provider stream success", move |recorder| {
            recorder.record_sync(event);
            crate::runtime::provider_profile_updates::mark_success(
                recorder.db.as_ref(),
                &provider_id,
                &runtime_mode,
                profile_id.as_deref(),
                Some(&model),
                Some(&request_id),
            );
        })
        .await;
}

async fn record_stream_closed_without_stop(
    recorder: &ProviderRuntimeEventRecorder,
    provider_id: &str,
    runtime_mode: &str,
    profile_id: Option<&str>,
    observed: &ObservedRequest,
    request_id: &str,
) {
    let event = base_event(
        provider_id,
        runtime_mode,
        ProviderRuntimeEventType::RequestFailed,
        ProviderRuntimeSeverity::Warn,
    )
    .with_request_id(request_id.to_string())
    .with_model(observed.model.clone())
    .with_reason("stream_closed_without_message_stop")
    .with_message("provider stream ended before message_stop")
    .with_redacted_json(serde_json::json!({
        "request_origin": observed.origin.as_deref(),
    }));
    let provider_id = provider_id.to_string();
    let runtime_mode = runtime_mode.to_string();
    let profile_id = profile_id.map(ToOwned::to_owned);
    let observed = observed.clone();
    let request_id = request_id.to_string();
    recorder
        .persist(
            "record provider stream closed without stop",
            move |recorder| {
                if let Some(event_id) = recorder.record_sync(event) {
                    crate::runtime::provider_incident_ledger::record_provider_incident(
                        crate::runtime::provider_incident_ledger::ProviderIncidentLedgerInput {
                            db: recorder.db.as_ref(),
                            agent_type: observed.agent_type.as_deref(),
                            agent_version: observed.agent_version.as_deref(),
                            run_id: observed.run_id.as_deref(),
                            model_id: &observed.model,
                            provider_id: &provider_id,
                            provider_event_id: &event_id,
                            reason_code: "stream_closed_without_message_stop",
                        },
                    );
                }
                crate::runtime::provider_profile_updates::mark_failure_reason(
                    recorder.db.as_ref(),
                    &provider_id,
                    &runtime_mode,
                    profile_id.as_deref(),
                    Some(&observed.model),
                    Some(&request_id),
                    "stream_closed_without_message_stop",
                );
            },
        )
        .await;
}
