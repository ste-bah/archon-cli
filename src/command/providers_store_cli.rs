//! Cozo-backed provider profile and rate-limit CLI surfaces.

use anyhow::Result;
use archon_learning::provider_auth_profiles::{
    ProviderAuthProfileRecord, get_provider_auth_profile, insert_provider_auth_profile,
    list_all_provider_auth_profiles, list_provider_auth_profiles,
};
use archon_learning::provider_rate_limits::list_provider_rate_limit_windows;
use archon_llm::providers::{list_compat, list_native};
use archon_llm::runtime::{AuthProfileSelection, AuthProfileSkipReason, AuthProfileSource};
use cozo::DbInstance;

pub(crate) fn render_provider_limits(provider_filter: Option<&str>) -> Result<String> {
    let db = open_learning_db()?;
    Ok(render_provider_limits_from_db(&db, provider_filter)?)
}

pub(crate) fn render_provider_profiles(provider_filter: Option<&str>) -> Result<String> {
    let db = open_learning_db()?;
    Ok(render_provider_profiles_from_db(&db, provider_filter)?)
}

pub(crate) fn render_provider_profile_inspect(profile_id: &str) -> Result<String> {
    let db = open_learning_db()?;
    let profile = get_provider_auth_profile(&db, profile_id)?
        .ok_or_else(|| anyhow::anyhow!("provider auth profile not found: {profile_id}"))?;
    Ok(render_profile_detail(&profile))
}

pub(crate) fn clear_provider_profile_cooldown(profile_id: &str) -> Result<String> {
    let db = open_learning_db()?;
    let mut profile = get_provider_auth_profile(&db, profile_id)?
        .ok_or_else(|| anyhow::anyhow!("provider auth profile not found: {profile_id}"))?;
    profile.cooldown_until = None;
    profile.disabled_reason = None;
    profile.updated_at = chrono::Utc::now().to_rfc3339();
    insert_provider_auth_profile(&db, &profile)?;
    Ok(format!(
        "Cleared cooldown for provider profile {} ({})\n",
        profile.profile_id, profile.provider_id
    ))
}

pub(crate) fn render_provider_profile_selection(
    provider_id: &str,
    auth_kinds: &[String],
    preferred_profile_id: Option<&str>,
) -> Result<String> {
    let db = open_learning_db()?;
    let allowed: Vec<&str> = auth_kinds.iter().map(String::as_str).collect();
    let report = crate::runtime::provider_auth_selection::select_provider_auth_profile_from_db(
        &db,
        provider_id,
        &allowed,
        preferred_profile_id,
    )?;
    Ok(render_profile_selection_report(&report, &allowed))
}

fn render_provider_limits_from_db(
    db: &DbInstance,
    provider_filter: Option<&str>,
) -> Result<String> {
    let mut windows = Vec::new();
    for provider_id in provider_ids(provider_filter) {
        windows.extend(list_provider_rate_limit_windows(db, &provider_id)?);
    }
    windows.sort_by(|a, b| {
        b.is_exhausted()
            .cmp(&a.is_exhausted())
            .then_with(|| b.observed_at.cmp(&a.observed_at))
    });

    let mut out = String::from("Provider rate limits (Cozo)\n\n");
    if windows.is_empty() {
        out.push_str("No provider rate-limit windows found.\n");
        return Ok(out);
    }
    out.push_str("provider             kind        used     resets_at             observed_at\n");
    out.push_str("----------------------------------------------------------------------------\n");
    for window in windows {
        out.push_str(&format!(
            "{:<20} {:<11} {:<8} {:<20} {}\n",
            window.provider_id,
            window.window_kind,
            percent_label(window.used_percent),
            window.resets_at.as_deref().unwrap_or("-"),
            window.observed_at,
        ));
    }
    Ok(out)
}

fn render_provider_profiles_from_db(
    db: &DbInstance,
    provider_filter: Option<&str>,
) -> Result<String> {
    let mut profiles = if let Some(provider_id) = provider_filter {
        list_provider_auth_profiles(db, provider_id)?
    } else {
        list_all_provider_auth_profiles(db)?
    };
    profiles.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));

    let mut out = String::from("Provider auth profiles (Cozo)\n\n");
    if profiles.is_empty() {
        out.push_str("No provider auth profiles found.\n");
        return Ok(out);
    }
    out.push_str("profile_id           provider             auth_kind   source       state\n");
    out.push_str("------------------------------------------------------------------------\n");
    for profile in profiles {
        out.push_str(&format!(
            "{:<20} {:<20} {:<11} {:<12} {}\n",
            profile.profile_id,
            profile.provider_id,
            profile.auth_kind,
            profile.source,
            profile_state(&profile),
        ));
    }
    Ok(out)
}

fn render_profile_detail(profile: &ProviderAuthProfileRecord) -> String {
    format!(
        "Provider auth profile\n\
         Profile:      {}\n\
         Provider:     {}\n\
         Auth kind:    {}\n\
         Display:      {}\n\
         Source:       {}\n\
         Fingerprint:  {}\n\
         Last used:    {}\n\
         Last good:    {}\n\
         Last failed:  {}\n\
         Failures:     {}\n\
         Cooldown:     {}\n\
         Disabled:     {}\n\
         Updated:      {}\n",
        profile.profile_id,
        profile.provider_id,
        profile.auth_kind,
        profile.display_name.as_deref().unwrap_or("-"),
        profile.source,
        profile.identity_fingerprint.as_deref().unwrap_or("-"),
        profile.last_used_at.as_deref().unwrap_or("-"),
        profile.last_good_at.as_deref().unwrap_or("-"),
        profile.last_failed_at.as_deref().unwrap_or("-"),
        profile.failure_count,
        profile.cooldown_until.as_deref().unwrap_or("-"),
        profile.disabled_reason.as_deref().unwrap_or("-"),
        profile.updated_at,
    )
}

fn render_profile_selection_report(
    report: &crate::runtime::provider_auth_selection::ProviderAuthSelectionReport,
    allowed_auth_kinds: &[&str],
) -> String {
    let mut out = String::from("Provider auth profile selection (Cozo)\n\n");
    out.push_str(&format!("Provider: {}\n", report.provider_id));
    out.push_str(&format!(
        "Allowed auth: {}\n",
        if allowed_auth_kinds.is_empty() {
            "any".into()
        } else {
            allowed_auth_kinds.join(", ")
        }
    ));
    match &report.selected {
        Some(selection) => out.push_str(&format!(
            "Selected: {} ({}/{})\n\n",
            selection.profile.profile_id,
            selection.profile.auth_kind,
            auth_profile_source_label(selection.profile.source)
        )),
        None => out.push_str("Selected: none\n\n"),
    }
    if report.ordered.is_empty() {
        out.push_str("No provider auth profiles found.\n");
        return out;
    }

    out.push_str("profile_id                         auth       state       reason\n");
    out.push_str("-----------------------------------------------------------------\n");
    for selection in &report.ordered {
        out.push_str(&format!(
            "{:<34} {:<10} {:<11} {}\n",
            selection.profile.profile_id,
            selection.profile.auth_kind,
            selection_state(selection, report),
            skip_reason_label(selection.reason),
        ));
    }
    out
}

fn provider_ids(provider_filter: Option<&str>) -> Vec<String> {
    if let Some(provider_id) = provider_filter {
        return vec![provider_id.to_string()];
    }
    let mut ids: Vec<String> = list_native()
        .into_iter()
        .chain(list_compat())
        .map(|descriptor| descriptor.id.clone())
        .collect();
    ids.sort();
    ids.dedup();
    ids
}

fn profile_state(profile: &ProviderAuthProfileRecord) -> &'static str {
    if profile.cooldown_until.is_some() {
        "cooldown"
    } else if profile.disabled_reason.is_some() {
        "disabled"
    } else if profile.failure_count > 0 {
        "degraded"
    } else {
        "ok"
    }
}

fn selection_state(
    selection: &AuthProfileSelection,
    report: &crate::runtime::provider_auth_selection::ProviderAuthSelectionReport,
) -> &'static str {
    if report
        .selected
        .as_ref()
        .map(|selected| selected.profile.profile_id.as_str())
        == Some(selection.profile.profile_id.as_str())
    {
        "selected"
    } else if selection.reason == AuthProfileSkipReason::Ok {
        "standby"
    } else {
        "skipped"
    }
}

fn skip_reason_label(reason: AuthProfileSkipReason) -> &'static str {
    match reason {
        AuthProfileSkipReason::Ok => "ok",
        AuthProfileSkipReason::ProfileMissing => "profile-missing",
        AuthProfileSkipReason::ProviderMismatch => "provider-mismatch",
        AuthProfileSkipReason::AuthKindMismatch => "auth-kind-mismatch",
        AuthProfileSkipReason::Expired => "expired",
        AuthProfileSkipReason::RefreshFailed => "refresh-failed",
        AuthProfileSkipReason::RateLimited => "rate-limited",
        AuthProfileSkipReason::UsageLimited => "usage-limited",
        AuthProfileSkipReason::Cooldown => "cooldown",
        AuthProfileSkipReason::Disabled => "disabled",
    }
}

fn auth_profile_source_label(source: AuthProfileSource) -> &'static str {
    match source {
        AuthProfileSource::ArchonStore => "archon-store",
        AuthProfileSource::Config => "config",
        AuthProfileSource::Env => "env",
        AuthProfileSource::ExternalCodex => "external-codex",
        AuthProfileSource::AwsChain => "aws-chain",
        AuthProfileSource::GcpCredentials => "gcp-credentials",
        AuthProfileSource::LocalRuntime => "local-runtime",
        AuthProfileSource::Unknown => "unknown",
    }
}

fn percent_label(value: Option<f64>) -> String {
    value
        .map(|value| format!("{value:.0}%"))
        .unwrap_or_else(|| "-".into())
}

fn open_learning_db() -> Result<DbInstance> {
    let db =
        crate::command::store_paths::open_evidence_db("learning", &["ARCHON_LEARNING_DB_PATH"])?;
    archon_learning::schema::ensure_learning_schema(&db)?;
    Ok(db)
}

#[cfg(test)]
mod tests {
    use super::*;
    use archon_learning::provider_rate_limits::ProviderRateLimitWindowRecord;

    fn test_db() -> DbInstance {
        let path = format!("/tmp/test-provider-store-cli-{}.db", uuid::Uuid::new_v4());
        let db = DbInstance::new("sqlite", &path, "").unwrap();
        archon_learning::schema::ensure_learning_schema(&db).unwrap();
        db
    }

    #[test]
    fn renders_profiles_from_cozo() {
        let db = test_db();
        insert_provider_auth_profile(
            &db,
            &ProviderAuthProfileRecord::new(
                "prof-1",
                "anthropic",
                "oauth",
                "archon_store",
                "2026-05-08T12:00:00Z",
            )
            .with_cooldown("2026-05-08T13:00:00Z", "rate_limited"),
        )
        .unwrap();

        let body = render_provider_profiles_from_db(&db, Some("anthropic")).unwrap();

        assert!(body.contains("prof-1"));
        assert!(body.contains("anthropic"));
        assert!(body.contains("cooldown"));
    }

    #[test]
    fn renders_custom_provider_profiles_without_filter() {
        let db = test_db();
        insert_provider_auth_profile(
            &db,
            &ProviderAuthProfileRecord::new(
                "prof-custom",
                "custom-openai-compatible",
                "api_key",
                "config",
                "2026-05-08T12:00:00Z",
            ),
        )
        .unwrap();

        let body = render_provider_profiles_from_db(&db, None).unwrap();

        assert!(body.contains("prof-custom"));
        assert!(body.contains("custom-openai-compatible"));
    }

    #[test]
    fn renders_profile_selection_skip_reasons() {
        let db = test_db();
        insert_provider_auth_profile(
            &db,
            &ProviderAuthProfileRecord::new(
                "anthropic-oauth",
                "anthropic",
                "oauth",
                "archon_store",
                "2026-05-08T12:00:00Z",
            ),
        )
        .unwrap();
        insert_provider_auth_profile(
            &db,
            &ProviderAuthProfileRecord::new(
                "anthropic-api",
                "anthropic",
                "api_key",
                "env",
                "2026-05-08T12:01:00Z",
            ),
        )
        .unwrap();
        let report = crate::runtime::provider_auth_selection::select_provider_auth_profile_from_db(
            &db,
            "anthropic",
            &["oauth"],
            None,
        )
        .unwrap();

        let body = render_profile_selection_report(&report, &["oauth"]);

        assert!(body.contains("Selected: anthropic-oauth"));
        assert!(body.contains("anthropic-api"));
        assert!(body.contains("auth-kind-mismatch"));
    }

    #[test]
    fn renders_exhausted_limits_first() {
        let db = test_db();
        archon_learning::provider_rate_limits::insert_provider_rate_limit_window(
            &db,
            &ProviderRateLimitWindowRecord::new(
                "limit-1",
                "openai-codex",
                "tokens",
                "2026-05-08T12:00:00Z",
            )
            .with_used_percent(50.0),
        )
        .unwrap();
        archon_learning::provider_rate_limits::insert_provider_rate_limit_window(
            &db,
            &ProviderRateLimitWindowRecord::new(
                "limit-2",
                "openai-codex",
                "usage",
                "2026-05-08T11:00:00Z",
            )
            .with_used_percent(100.0),
        )
        .unwrap();

        let body = render_provider_limits_from_db(&db, Some("openai-codex")).unwrap();

        let usage = body.find("usage").unwrap();
        let tokens = body.find("tokens").unwrap();
        assert!(usage < tokens);
    }
}
