//! Persist provider rate and usage-limit windows observed from real calls.

use std::sync::Arc;

use archon_learning::provider_rate_limits::{
    ProviderRateLimitWindowRecord, insert_provider_rate_limit_window,
};
use archon_llm::provider::LlmError;
use chrono::{DateTime, Duration, Utc};
use cozo::DbInstance;
use sha2::{Digest, Sha256};

pub(crate) fn record_limit_window(
    db: Option<&Arc<DbInstance>>,
    provider_id: &str,
    model_id: Option<&str>,
    error: &LlmError,
) {
    let Some(db) = db else {
        return;
    };
    let Some(window) = build_limit_window(provider_id, model_id, error) else {
        return;
    };
    if let Err(error) = insert_provider_rate_limit_window(db, &window) {
        tracing::warn!(
            %error,
            provider = %provider_id,
            "provider rate-limit window persistence failed"
        );
    }
}

fn build_limit_window(
    provider_id: &str,
    model_id: Option<&str>,
    error: &LlmError,
) -> Option<ProviderRateLimitWindowRecord> {
    let observed_at = Utc::now();
    let (kind, limit_id, limit_name, reset_hint, raw) = match error {
        LlmError::RateLimited { retry_after_secs } => (
            "rate_limit",
            "retry_after",
            "Provider rate limit",
            reset_after_secs(observed_at, *retry_after_secs),
            serde_json::json!({
                "source": "llm_error",
                "error_kind": "rate_limited",
                "retry_after_secs": retry_after_secs,
            }),
        ),
        LlmError::QuotaExceeded(message) => {
            let reset_hint = reset_hint_from_text(message, observed_at);
            (
                "usage_limit",
                "quota_or_usage",
                "Provider usage limit",
                reset_hint,
                serde_json::json!({
                    "source": "llm_error",
                    "error_kind": "quota_exceeded",
                    "message_sha256": message_hash(message),
                    "reset_hint_found": reset_hint.is_some(),
                }),
            )
        }
        _ => return None,
    };

    let mut window = ProviderRateLimitWindowRecord::new(
        format!("provider-limit-{}", uuid::Uuid::new_v4()),
        provider_id.to_string(),
        kind,
        observed_at.to_rfc3339(),
    )
    .with_limit(limit_id, limit_name)
    .with_used_percent(100.0)
    .with_redacted_json(raw);
    if let Some(model_id) = model_id.filter(|value| !value.trim().is_empty()) {
        window = window.with_model(model_id.to_string());
    }
    if let Some(reset_hint) = reset_hint {
        window = window.with_resets_at(reset_hint.to_rfc3339());
    }
    Some(window)
}

fn reset_after_secs(observed_at: DateTime<Utc>, retry_after_secs: u64) -> Option<DateTime<Utc>> {
    if retry_after_secs == 0 {
        None
    } else {
        Some(observed_at + Duration::seconds(retry_after_secs as i64))
    }
}

fn reset_hint_from_text(message: &str, observed_at: DateTime<Utc>) -> Option<DateTime<Utc>> {
    rfc3339_hint(message).or_else(|| relative_duration_hint(message, observed_at))
}

fn rfc3339_hint(message: &str) -> Option<DateTime<Utc>> {
    message
        .split_ascii_whitespace()
        .map(|token| {
            token
                .trim_matches(|c: char| matches!(c, '"' | '\'' | ',' | ';' | ')' | '(' | '[' | ']'))
        })
        .find_map(|token| {
            DateTime::parse_from_rfc3339(token)
                .ok()
                .map(|dt| dt.with_timezone(&Utc))
        })
}

fn relative_duration_hint(message: &str, observed_at: DateTime<Utc>) -> Option<DateTime<Utc>> {
    let tokens: Vec<String> = message
        .to_ascii_lowercase()
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|token| !token.is_empty())
        .map(ToOwned::to_owned)
        .collect();

    for pair in tokens.windows(2) {
        let Ok(amount) = pair[0].parse::<i64>() else {
            continue;
        };
        let Some(seconds) = unit_seconds(&pair[1]).map(|unit| amount.saturating_mul(unit)) else {
            continue;
        };
        if seconds > 0 {
            return Some(observed_at + Duration::seconds(seconds));
        }
    }
    None
}

fn unit_seconds(unit: &str) -> Option<i64> {
    match unit {
        "second" | "seconds" | "sec" | "secs" => Some(1),
        "minute" | "minutes" | "min" | "mins" => Some(60),
        "hour" | "hours" | "hr" | "hrs" => Some(60 * 60),
        "day" | "days" => Some(24 * 60 * 60),
        _ => None,
    }
}

fn message_hash(message: &str) -> String {
    hex::encode(Sha256::digest(message.as_bytes()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_db() -> Arc<DbInstance> {
        let path = format!(
            "/tmp/test-provider-limit-windows-{}.db",
            uuid::Uuid::new_v4()
        );
        let db = DbInstance::new("sqlite", &path, "").unwrap();
        archon_learning::schema::ensure_learning_schema(&db).unwrap();
        Arc::new(db)
    }

    #[test]
    fn rate_limited_error_persists_exhausted_window() {
        let db = test_db();
        record_limit_window(
            Some(&db),
            "anthropic",
            Some("claude-sonnet-4-6"),
            &LlmError::RateLimited {
                retry_after_secs: 90,
            },
        );

        let windows = archon_learning::provider_rate_limits::list_provider_rate_limit_windows(
            &db,
            "anthropic",
        )
        .unwrap();

        assert_eq!(windows.len(), 1);
        assert_eq!(windows[0].window_kind, "rate_limit");
        assert_eq!(windows[0].used_percent, Some(100.0));
        assert_eq!(windows[0].model_id.as_deref(), Some("claude-sonnet-4-6"));
        assert!(windows[0].resets_at.is_some());
    }

    #[test]
    fn quota_error_persists_hash_not_raw_message() {
        let db = test_db();
        let message = "usage limit reached for sk-ant-api03-secret; resets in 2 hours";
        record_limit_window(
            Some(&db),
            "openai-codex",
            Some("gpt-5.4"),
            &LlmError::QuotaExceeded(message.to_string()),
        );

        let windows = archon_learning::provider_rate_limits::list_provider_rate_limit_windows(
            &db,
            "openai-codex",
        )
        .unwrap();
        let raw = windows[0].raw_redacted_json.to_string();

        assert_eq!(windows[0].window_kind, "usage_limit");
        assert!(windows[0].resets_at.is_some());
        assert!(windows[0].raw_redacted_json["message_sha256"].is_string());
        assert!(!raw.contains("sk-ant-api03-secret"));
    }

    #[test]
    fn reset_hint_parses_rfc3339_and_relative_units() {
        let observed = DateTime::parse_from_rfc3339("2026-05-08T12:00:00Z")
            .unwrap()
            .with_timezone(&Utc);

        let absolute = reset_hint_from_text("resets at 2026-05-09T00:00:00Z", observed).unwrap();
        let relative = reset_hint_from_text("try again in 15 minutes", observed).unwrap();

        assert_eq!(absolute.to_rfc3339(), "2026-05-09T00:00:00+00:00");
        assert_eq!(relative.to_rfc3339(), "2026-05-08T12:15:00+00:00");
    }
}
