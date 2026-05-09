//! Forward provider stream events while recording terminal runtime outcomes.

use archon_llm::runtime::{ProviderRuntimeEventType, ProviderRuntimeSeverity};
use archon_llm::streaming::StreamEvent;
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
        while let Some(event) = inner_rx.recv().await {
            match &event {
                StreamEvent::Error {
                    error_type,
                    message: _,
                } => {
                    record_stream_error(
                        &recorder,
                        &provider_id,
                        &runtime_mode,
                        profile_id.as_deref(),
                        &observed,
                        &request_id,
                        error_type,
                    );
                }
                StreamEvent::MessageStop => {
                    completed = true;
                    record_stream_success(
                        &recorder,
                        &provider_id,
                        &runtime_mode,
                        profile_id.as_deref(),
                        &observed,
                        &request_id,
                    );
                }
                _ => {}
            }
            if tx.send(event).await.is_err() {
                break;
            }
        }
        if !completed {
            record_stream_closed_without_stop(
                &recorder,
                &provider_id,
                &runtime_mode,
                profile_id.as_deref(),
                &observed,
                &request_id,
            );
        }
    });
    rx
}

fn record_stream_error(
    recorder: &ProviderRuntimeEventRecorder,
    provider_id: &str,
    runtime_mode: &str,
    profile_id: Option<&str>,
    observed: &ObservedRequest,
    request_id: &str,
    error_type: &str,
) {
    recorder.record(
        base_event(
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
        })),
    );
    crate::runtime::provider_profile_updates::mark_failure_reason(
        recorder.db.as_ref(),
        provider_id,
        runtime_mode,
        profile_id,
        Some(&observed.model),
        Some(request_id),
        error_type,
    );
}

fn record_stream_success(
    recorder: &ProviderRuntimeEventRecorder,
    provider_id: &str,
    runtime_mode: &str,
    profile_id: Option<&str>,
    observed: &ObservedRequest,
    request_id: &str,
) {
    recorder.record(
        base_event(
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
        })),
    );
    crate::runtime::provider_profile_updates::mark_success(
        recorder.db.as_ref(),
        provider_id,
        runtime_mode,
        profile_id,
        Some(&observed.model),
        Some(request_id),
    );
}

fn record_stream_closed_without_stop(
    recorder: &ProviderRuntimeEventRecorder,
    provider_id: &str,
    runtime_mode: &str,
    profile_id: Option<&str>,
    observed: &ObservedRequest,
    request_id: &str,
) {
    recorder.record(
        base_event(
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
        })),
    );
    crate::runtime::provider_profile_updates::mark_failure_reason(
        recorder.db.as_ref(),
        provider_id,
        runtime_mode,
        profile_id,
        Some(&observed.model),
        Some(request_id),
        "stream_closed_without_message_stop",
    );
}
