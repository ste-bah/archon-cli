//! Connection establishment is outage time; an answered stream owns its normal clocks.
use archon_llm::provider::{LlmError, LlmProvider, LlmRequest};
use archon_llm::{provider_fault, streaming::StreamEvent};
use archon_tools::provider_recovery::ProviderAttempt;
use tokio::sync::mpsc::Receiver;

pub(crate) async fn open(
    provider: &dyn LlmProvider,
    request: LlmRequest,
) -> Result<Receiver<StreamEvent>, LlmError> {
    if !archon_tools::provider_recovery::enabled() {
        return provider.stream(request).await;
    }
    provider_fault::begin_request();
    let mut response = provider_fault::response_watch();
    let mut accounting = ProviderAttempt::start_current();
    let work = provider.stream(request);
    tokio::pin!(work);
    let result = loop {
        tokio::select! {
            biased;
            result = &mut work => break result,
            _ = async {
                match &mut response {
                    Some(response) => { let _ = response.changed().await; }
                    None => std::future::pending().await,
                }
            } => {
                if provider_fault::pending_response().is_some_and(|(status, _)| status < 400) {
                    drop(accounting.take());
                }
            }
        }
    };
    // Some interactive adapters represent body-read expiry as an idle receiver.
    // Its known error status must remain typed on the durable workflow path.
    let result = match (result, provider_fault::pending_response()) {
        (Ok(_), Some((status, _))) if status >= 400 => Err(provider_fault::opening_timeout()),
        (Err(LlmError::Http(message)), Some((status, hint)))
            if status >= 400 && message.leading_status().is_none() =>
        {
            // A failed error-body read must not erase the status or header
            // already received. Keep existing typed request rejections intact.
            Err(LlmError::Server {
                status,
                message,
                pipeline_retry_after_secs: hint,
            })
        }
        (result, _) => result,
    };
    provider_fault::begin_request();
    drop(accounting);
    result
}

tokio::task_local! { static SUMMARY_IDLE: std::time::Duration; }

pub(crate) async fn with_idle_guard<T>(
    limit: std::time::Duration,
    work: impl std::future::Future<Output = T>,
) -> T {
    SUMMARY_IDLE.scope(limit, work).await
}

fn summary_idle() -> std::time::Duration {
    SUMMARY_IDLE.try_with(|limit| *limit).unwrap_or_else(|_| {
        std::time::Duration::from_secs(crate::config::DEFAULT_STREAM_IDLE_TIMEOUT_SECS)
    })
}

pub(super) async fn open_summary(
    provider: &dyn LlmProvider,
    request: LlmRequest,
) -> Result<Receiver<StreamEvent>, LlmError> {
    if !archon_tools::provider_recovery::enabled() {
        return open(provider, request).await;
    }
    crate::subagent::runner::stream_idle_window::within(summary_idle(), open(provider, request))
        .await
        .unwrap_or_else(|_| Err(provider_fault::opening_timeout()))
}

pub(super) async fn receive_summary(
    receiver: &mut Receiver<StreamEvent>,
) -> Result<Option<StreamEvent>, LlmError> {
    if !archon_tools::provider_recovery::enabled() {
        return Ok(receiver.recv().await);
    }
    crate::subagent::runner::stream_idle_window::within(summary_idle(), receiver.recv())
        .await
        .map_err(|_| {
            LlmError::Http("provider transport stalled: summary stream idle timeout".into())
        })
}
