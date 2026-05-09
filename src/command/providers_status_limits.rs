//! Rate-limit window enrichment for provider status.

use anyhow::Result;
use archon_learning::provider_rate_limits::ProviderRateLimitWindowRecord;
use archon_llm::runtime::{ProviderRateLimitWindow, RateLimitWindowKind, redact_provider_metadata};
use chrono::{DateTime, Utc};
use cozo::DbInstance;

pub(crate) fn recent_rate_limits_from_db(
    db: &DbInstance,
    provider_id: &str,
    now: DateTime<Utc>,
) -> Result<Vec<ProviderRateLimitWindow>> {
    let mut limits: Vec<_> =
        archon_learning::provider_rate_limits::list_provider_rate_limit_windows(db, provider_id)?
            .into_iter()
            .filter_map(rate_limit_from_record)
            .filter(|limit| limit.is_recent(now))
            .collect();
    limits.sort_by(|a, b| {
        b.is_exhausted()
            .cmp(&a.is_exhausted())
            .then_with(|| b.observed_at.cmp(&a.observed_at))
    });
    limits.truncate(3);
    Ok(limits)
}

fn rate_limit_from_record(
    record: ProviderRateLimitWindowRecord,
) -> Option<ProviderRateLimitWindow> {
    let observed_at = parse_utc(&record.observed_at)?;
    let resets_at = record.resets_at.as_deref().and_then(parse_utc);
    Some(ProviderRateLimitWindow {
        id: record.window_id,
        provider_id: record.provider_id,
        profile_id: record.profile_id,
        model_id: record.model_id,
        limit_id: record.limit_id,
        limit_name: record.limit_name,
        window_kind: rate_limit_kind(&record.window_kind),
        used_percent: record.used_percent.map(|value| value.clamp(0.0, 100.0)),
        resets_at,
        raw_redacted_json: redact_provider_metadata(record.raw_redacted_json),
        observed_at,
    })
}

fn parse_utc(value: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(value)
        .ok()
        .map(|value| value.with_timezone(&Utc))
}

fn rate_limit_kind(value: &str) -> RateLimitWindowKind {
    match value {
        "requests" | "request" | "rate_limit" => RateLimitWindowKind::Requests,
        "tokens" | "token" => RateLimitWindowKind::Tokens,
        "spend" | "cost" => RateLimitWindowKind::Spend,
        "usage" | "usage_limit" => RateLimitWindowKind::Usage,
        _ => RateLimitWindowKind::Unknown,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use archon_learning::provider_rate_limits::insert_provider_rate_limit_window;

    fn test_db() -> DbInstance {
        let path = format!(
            "/tmp/test-provider-status-limits-{}.db",
            uuid::Uuid::new_v4()
        );
        let db = DbInstance::new("sqlite", &path, "").unwrap();
        archon_learning::schema::ensure_learning_schema(&db).unwrap();
        db
    }

    #[test]
    fn recent_rate_limits_return_exhausted_windows_first() {
        let db = test_db();
        let now = Utc::now();
        insert_provider_rate_limit_window(
            &db,
            &ProviderRateLimitWindowRecord::new(
                "old",
                "openai-codex",
                "usage_limit",
                (now - chrono::Duration::minutes(20)).to_rfc3339(),
            )
            .with_used_percent(100.0),
        )
        .unwrap();
        insert_provider_rate_limit_window(
            &db,
            &ProviderRateLimitWindowRecord::new(
                "recent-low",
                "openai-codex",
                "rate_limit",
                (now - chrono::Duration::minutes(1)).to_rfc3339(),
            )
            .with_used_percent(50.0),
        )
        .unwrap();
        insert_provider_rate_limit_window(
            &db,
            &ProviderRateLimitWindowRecord::new(
                "recent-full",
                "openai-codex",
                "usage_limit",
                now.to_rfc3339(),
            )
            .with_used_percent(100.0),
        )
        .unwrap();

        let limits = recent_rate_limits_from_db(&db, "openai-codex", now).unwrap();

        assert_eq!(limits.len(), 2);
        assert_eq!(limits[0].id, "recent-full");
        assert_eq!(limits[0].window_kind, RateLimitWindowKind::Usage);
        assert!(limits[0].is_exhausted());
    }
}
