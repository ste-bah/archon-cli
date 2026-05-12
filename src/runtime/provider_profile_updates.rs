//! Update Cozo provider auth profiles from observed runtime outcomes.

use std::sync::Arc;

use archon_learning::provider_auth_profiles::{
    get_provider_auth_profile, insert_provider_auth_profile,
};
use archon_llm::provider::LlmError;
use archon_llm::runtime::{
    ProviderRuntimeEvent, ProviderRuntimeEventType, ProviderRuntimeSeverity,
};
use chrono::{DateTime, Utc};
use cozo::DbInstance;

use crate::runtime::provider_event_record::provider_event_record;

pub(crate) fn mark_success(
    db: Option<&Arc<DbInstance>>,
    provider_id: &str,
    runtime_mode: &str,
    profile_id: Option<&str>,
    model_id: Option<&str>,
    request_id: Option<&str>,
) {
    let Some((db, profile_id)) = db.zip(profile_id) else {
        return;
    };
    let Ok(Some(mut profile)) = get_provider_auth_profile(db, profile_id) else {
        return;
    };
    if profile.provider_id != provider_id {
        return;
    }
    let had_cooldown = profile.cooldown_until.is_some();
    let now = Utc::now().to_rfc3339();
    profile.last_used_at = Some(now.clone());
    profile.last_good_at = Some(now.clone());
    profile.failure_count = 0;
    profile.cooldown_until = None;
    profile.disabled_reason = None;
    profile.updated_at = now;
    if insert_provider_auth_profile(db, &profile).is_err() {
        return;
    }
    if had_cooldown {
        record_cooldown_event(
            db,
            provider_id,
            runtime_mode,
            profile_id,
            model_id,
            request_id,
            ProviderRuntimeEventType::ProfileCooldownCleared,
            "profile_success",
            None,
        );
    }
}

pub(crate) fn mark_failure(
    db: Option<&Arc<DbInstance>>,
    provider_id: &str,
    runtime_mode: &str,
    profile_id: Option<&str>,
    model_id: Option<&str>,
    request_id: Option<&str>,
    error: &LlmError,
) {
    let reason = error_reason(error);
    let cooldown = cooldown_until(error, Utc::now());
    mark_failure_inner(
        db,
        provider_id,
        runtime_mode,
        profile_id,
        model_id,
        request_id,
        reason,
        cooldown,
    );
}

pub(crate) fn mark_failure_reason(
    db: Option<&Arc<DbInstance>>,
    provider_id: &str,
    runtime_mode: &str,
    profile_id: Option<&str>,
    model_id: Option<&str>,
    request_id: Option<&str>,
    reason: &str,
) {
    mark_failure_inner(
        db,
        provider_id,
        runtime_mode,
        profile_id,
        model_id,
        request_id,
        reason,
        None,
    );
}

fn mark_failure_inner(
    db: Option<&Arc<DbInstance>>,
    provider_id: &str,
    runtime_mode: &str,
    profile_id: Option<&str>,
    model_id: Option<&str>,
    request_id: Option<&str>,
    reason: &str,
    cooldown: Option<DateTime<Utc>>,
) {
    let Some((db, profile_id)) = db.zip(profile_id) else {
        return;
    };
    let Ok(Some(mut profile)) = get_provider_auth_profile(db, profile_id) else {
        return;
    };
    if profile.provider_id != provider_id {
        return;
    }
    let now = Utc::now().to_rfc3339();
    profile.last_used_at = Some(now.clone());
    profile.last_failed_at = Some(now.clone());
    profile.failure_count = profile.failure_count.saturating_add(1);
    profile.updated_at = now;
    if let Some(cooldown) = cooldown {
        profile.cooldown_until = Some(cooldown.to_rfc3339());
        profile.disabled_reason = Some(reason.to_string());
    }
    if insert_provider_auth_profile(db, &profile).is_err() {
        return;
    }
    if let Some(cooldown) = cooldown {
        record_cooldown_event(
            db,
            provider_id,
            runtime_mode,
            profile_id,
            model_id,
            request_id,
            ProviderRuntimeEventType::ProfileCooldownStarted,
            reason,
            Some(cooldown),
        );
    }
}

fn cooldown_until(error: &LlmError, now: DateTime<Utc>) -> Option<DateTime<Utc>> {
    match error {
        LlmError::RateLimited { retry_after_secs } => {
            crate::runtime::provider_limit_windows::reset_after_secs(now, *retry_after_secs)
        }
        LlmError::QuotaExceeded(message) => {
            crate::runtime::provider_limit_windows::reset_hint_from_text(message, now)
        }
        _ => None,
    }
}

fn error_reason(error: &LlmError) -> &'static str {
    match error {
        LlmError::RateLimited { .. } => "rate_limited",
        LlmError::QuotaExceeded(_) => "quota_exceeded",
        LlmError::Auth(_) => "auth_error",
        LlmError::Overloaded => "overloaded",
        LlmError::Server { .. } => "server_error",
        LlmError::Unsupported(_) => "unsupported_feature",
        LlmError::ProviderNotFound { .. } => "provider_not_found",
        LlmError::Aborted => "aborted",
        LlmError::Http(_) => "http_error",
        LlmError::Serialize(_) => "serialization_error",
        LlmError::ContextWindowExceeded { .. } => "context_window_exceeded",
        _ => "unknown_error",
    }
}

#[allow(clippy::too_many_arguments)]
fn record_cooldown_event(
    db: &DbInstance,
    provider_id: &str,
    runtime_mode: &str,
    profile_id: &str,
    model_id: Option<&str>,
    request_id: Option<&str>,
    event_type: ProviderRuntimeEventType,
    reason: &str,
    cooldown_until: Option<DateTime<Utc>>,
) {
    let mut event = ProviderRuntimeEvent::new(
        provider_id,
        runtime_mode,
        event_type,
        ProviderRuntimeSeverity::Warn,
    )
    .with_profile(profile_id.to_string())
    .with_reason(reason)
    .with_redacted_json(serde_json::json!({
        "cooldown_until": cooldown_until.map(|value| value.to_rfc3339()),
    }));
    if let Some(model_id) = model_id {
        event = event.with_model(model_id.to_string());
    }
    if let Some(request_id) = request_id {
        event = event.with_request_id(request_id.to_string());
    }
    let record = provider_event_record(event);
    if let Err(error) = archon_learning::runtime_events::insert_provider_runtime_event(db, &record)
    {
        tracing::warn!(%error, provider = %provider_id, profile = %profile_id, "profile cooldown event persistence failed");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use archon_learning::provider_auth_profiles::ProviderAuthProfileRecord;

    fn test_db() -> Arc<DbInstance> {
        let path = format!(
            "/tmp/test-provider-profile-updates-{}.db",
            uuid::Uuid::new_v4()
        );
        let db = DbInstance::new("sqlite", &path, "").unwrap();
        archon_learning::schema::ensure_learning_schema(&db).unwrap();
        Arc::new(db)
    }

    fn insert_profile(db: &DbInstance) {
        insert_provider_auth_profile(
            db,
            &ProviderAuthProfileRecord::new(
                "anthropic-oauth",
                "anthropic",
                "oauth",
                "archon_store",
                "2026-05-09T12:00:00Z",
            ),
        )
        .unwrap();
    }

    #[test]
    fn success_updates_profile_and_clears_cooldown() {
        let db = test_db();
        insert_provider_auth_profile(
            &db,
            &ProviderAuthProfileRecord::new(
                "anthropic-oauth",
                "anthropic",
                "oauth",
                "archon_store",
                "2026-05-09T12:00:00Z",
            )
            .with_cooldown("2026-05-09T13:00:00Z", "rate_limited"),
        )
        .unwrap();

        mark_success(
            Some(&db),
            "anthropic",
            "direct",
            Some("anthropic-oauth"),
            Some("claude-sonnet-4-6"),
            Some("req-1"),
        );

        let profile = get_provider_auth_profile(&db, "anthropic-oauth")
            .unwrap()
            .unwrap();
        assert_eq!(profile.failure_count, 0);
        assert!(profile.last_good_at.is_some());
        assert!(profile.cooldown_until.is_none());
        let events =
            archon_learning::runtime_events::list_provider_runtime_events(&db, Some("anthropic"))
                .unwrap();
        assert_eq!(events[0].event_type, "profile_cooldown_cleared");
    }

    #[test]
    fn rate_limit_failure_starts_profile_cooldown() {
        let db = test_db();
        insert_profile(&db);

        mark_failure(
            Some(&db),
            "anthropic",
            "direct",
            Some("anthropic-oauth"),
            Some("claude-sonnet-4-6"),
            Some("req-1"),
            &LlmError::RateLimited {
                retry_after_secs: 60,
            },
        );

        let profile = get_provider_auth_profile(&db, "anthropic-oauth")
            .unwrap()
            .unwrap();
        assert_eq!(profile.failure_count, 1);
        assert_eq!(profile.disabled_reason.as_deref(), Some("rate_limited"));
        assert!(profile.cooldown_until.is_some());
        let events =
            archon_learning::runtime_events::list_provider_runtime_events(&db, Some("anthropic"))
                .unwrap();
        assert_eq!(events[0].event_type, "profile_cooldown_started");
    }

    #[test]
    fn generic_failure_does_not_start_cooldown() {
        let db = test_db();
        insert_profile(&db);

        mark_failure_reason(
            Some(&db),
            "anthropic",
            "direct",
            Some("anthropic-oauth"),
            None,
            None,
            "stream_error",
        );

        let profile = get_provider_auth_profile(&db, "anthropic-oauth")
            .unwrap()
            .unwrap();
        assert_eq!(profile.failure_count, 1);
        assert!(profile.cooldown_until.is_none());
    }
}
