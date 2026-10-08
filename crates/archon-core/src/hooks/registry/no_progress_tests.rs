use super::HookRegistry;
use crate::hooks::{HookCallbackEntry, HookEvent, HookOutcome, HookResult, SourceAuthority};
use std::path::Path;
use std::sync::Arc;

#[tokio::test]
async fn callback_errors_cannot_claim_harness_no_progress_stops() {
    let registry = HookRegistry::new();
    for (index, reason) in [
        "routine callback error",
        "no progress mentioned by callback",
        "callback stopped by application code",
    ]
    .into_iter()
    .enumerate()
    {
        let reason = reason.to_owned();
        registry.register_callback(
            HookEvent::PostToolUse,
            HookCallbackEntry {
                name: format!("callback-{index}"),
                callback: Arc::new(move |_| HookResult {
                    outcome: HookOutcome::NonBlockingError,
                    reason: Some(reason.clone()),
                    ..Default::default()
                }),
                authority: SourceAuthority::User,
                timeout_secs: 5,
            },
        );
    }
    let aggregate = registry
        .execute_hooks(
            HookEvent::PostToolUse,
            serde_json::json!({}),
            Path::new("."),
            "marker-test",
        )
        .await;
    assert_eq!(aggregate.nonblocking_errors.len(), 3);
    assert!(aggregate.no_progress_stops.is_empty());
}
