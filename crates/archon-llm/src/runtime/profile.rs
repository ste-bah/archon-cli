use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuthProfileSource {
    ArchonStore,
    Config,
    Env,
    ExternalCodex,
    AwsChain,
    GcpCredentials,
    LocalRuntime,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuthProfileSkipReason {
    Ok,
    ProfileMissing,
    ProviderMismatch,
    AuthKindMismatch,
    Expired,
    RefreshFailed,
    RateLimited,
    UsageLimited,
    Cooldown,
    Disabled,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProviderAuthProfile {
    pub profile_id: String,
    pub provider_id: String,
    pub auth_kind: String,
    pub display_name: Option<String>,
    pub source: AuthProfileSource,
    pub account_id: Option<String>,
    pub identity_fingerprint: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub last_used_at: Option<DateTime<Utc>>,
    pub last_good_at: Option<DateTime<Utc>>,
    pub last_failed_at: Option<DateTime<Utc>>,
    pub failure_count: u32,
    pub cooldown_until: Option<DateTime<Utc>>,
    pub disabled_reason: Option<String>,
    pub metadata_json: Value,
}

impl ProviderAuthProfile {
    pub fn new(
        profile_id: impl Into<String>,
        provider_id: impl Into<String>,
        auth_kind: impl Into<String>,
        source: AuthProfileSource,
    ) -> Self {
        let now = Utc::now();
        Self {
            profile_id: profile_id.into(),
            provider_id: provider_id.into(),
            auth_kind: auth_kind.into(),
            display_name: None,
            source,
            account_id: None,
            identity_fingerprint: None,
            created_at: now,
            updated_at: now,
            last_used_at: None,
            last_good_at: None,
            last_failed_at: None,
            failure_count: 0,
            cooldown_until: None,
            disabled_reason: None,
            metadata_json: Value::Object(Default::default()),
        }
    }

    pub fn with_display_name(mut self, display_name: impl Into<String>) -> Self {
        self.display_name = Some(display_name.into());
        self
    }

    pub fn with_account_id(mut self, account_id: impl Into<String>) -> Self {
        self.account_id = Some(account_id.into());
        self
    }

    pub fn with_fingerprint(mut self, fingerprint: impl Into<String>) -> Self {
        self.identity_fingerprint = Some(fingerprint.into());
        self
    }

    pub fn with_metadata(mut self, metadata_json: Value) -> Self {
        self.metadata_json = metadata_json;
        self
    }

    pub fn mark_success(&mut self, now: DateTime<Utc>) {
        self.last_used_at = Some(now);
        self.last_good_at = Some(now);
        self.updated_at = now;
        self.failure_count = 0;
        self.cooldown_until = None;
    }

    pub fn mark_failure(&mut self, now: DateTime<Utc>) {
        self.last_used_at = Some(now);
        self.last_failed_at = Some(now);
        self.updated_at = now;
        self.failure_count = self.failure_count.saturating_add(1);
    }

    pub fn start_cooldown(&mut self, until: DateTime<Utc>, reason: impl Into<String>) {
        self.cooldown_until = Some(until);
        self.disabled_reason = Some(reason.into());
        self.updated_at = Utc::now();
    }

    pub fn disable(&mut self, reason: impl Into<String>, now: DateTime<Utc>) {
        self.disabled_reason = Some(reason.into());
        self.updated_at = now;
    }

    pub fn is_in_cooldown(&self, now: DateTime<Utc>) -> bool {
        self.cooldown_until
            .map(|cooldown_until| cooldown_until > now)
            .unwrap_or(false)
    }

    pub fn skip_reason_for(
        &self,
        provider_id: &str,
        allowed_auth_kinds: &[&str],
        now: DateTime<Utc>,
    ) -> AuthProfileSkipReason {
        if self.disabled_reason.is_some() && !self.is_in_cooldown(now) {
            return AuthProfileSkipReason::Disabled;
        }
        if self.provider_id != provider_id {
            return AuthProfileSkipReason::ProviderMismatch;
        }
        if !allowed_auth_kinds.is_empty()
            && !allowed_auth_kinds
                .iter()
                .any(|allowed| *allowed == self.auth_kind)
        {
            return AuthProfileSkipReason::AuthKindMismatch;
        }
        if self.is_in_cooldown(now) {
            return AuthProfileSkipReason::Cooldown;
        }
        AuthProfileSkipReason::Ok
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AuthProfileSelection {
    pub profile: ProviderAuthProfile,
    pub reason: AuthProfileSkipReason,
}

pub fn ordered_profiles_for_selection(
    profiles: &[ProviderAuthProfile],
    provider_id: &str,
    allowed_auth_kinds: &[&str],
    preferred_profile_id: Option<&str>,
    now: DateTime<Utc>,
) -> Vec<AuthProfileSelection> {
    let mut selections: Vec<AuthProfileSelection> = profiles
        .iter()
        .cloned()
        .map(|profile| {
            let reason = profile.skip_reason_for(provider_id, allowed_auth_kinds, now);
            AuthProfileSelection { profile, reason }
        })
        .collect();

    selections.sort_by(|a, b| {
        selection_rank(a, preferred_profile_id, now)
            .cmp(&selection_rank(b, preferred_profile_id, now))
            .then_with(|| a.profile.profile_id.cmp(&b.profile.profile_id))
    });

    selections
}

fn selection_rank(
    selection: &AuthProfileSelection,
    preferred_profile_id: Option<&str>,
    now: DateTime<Utc>,
) -> (u8, i64) {
    let preferred = preferred_profile_id == Some(selection.profile.profile_id.as_str());
    let class = match selection.reason {
        AuthProfileSkipReason::Ok if preferred => 0,
        AuthProfileSkipReason::Ok => 1,
        AuthProfileSkipReason::Cooldown => 2,
        AuthProfileSkipReason::Disabled => 3,
        _ => 4,
    };
    let last_used = selection
        .profile
        .last_used_at
        .map(|last_used| last_used.timestamp())
        .unwrap_or(i64::MIN);
    let last_used = if class == 2 {
        selection
            .profile
            .cooldown_until
            .map(|cooldown_until| cooldown_until.timestamp())
            .unwrap_or_else(|| now.timestamp())
    } else {
        last_used
    };
    (class, last_used)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Duration;

    fn profile(id: &str) -> ProviderAuthProfile {
        ProviderAuthProfile::new(id, "anthropic", "oauth", AuthProfileSource::ArchonStore)
    }

    #[test]
    fn healthy_preferred_profile_sorts_first() {
        let now = Utc::now();
        let mut old = profile("old");
        old.last_used_at = Some(now - Duration::hours(2));
        let mut preferred = profile("preferred");
        preferred.last_used_at = Some(now - Duration::minutes(10));

        let ordered = ordered_profiles_for_selection(
            &[old, preferred],
            "anthropic",
            &["oauth"],
            Some("preferred"),
            now,
        );

        assert_eq!(ordered[0].profile.profile_id, "preferred");
        assert_eq!(ordered[0].reason, AuthProfileSkipReason::Ok);
    }

    #[test]
    fn cooldown_and_disabled_profiles_sort_after_healthy_profiles() {
        let now = Utc::now();
        let healthy = profile("healthy");
        let mut cooldown = profile("cooldown");
        cooldown.start_cooldown(now + Duration::minutes(20), "rate_limited");
        let mut disabled = profile("disabled");
        disabled.disable("revoked", now);

        let ordered = ordered_profiles_for_selection(
            &[disabled, cooldown, healthy],
            "anthropic",
            &["oauth"],
            None,
            now,
        );

        assert_eq!(ordered[0].profile.profile_id, "healthy");
        assert_eq!(ordered[1].reason, AuthProfileSkipReason::Cooldown);
        assert_eq!(ordered[2].reason, AuthProfileSkipReason::Disabled);
    }

    #[test]
    fn provider_and_auth_mismatch_are_explicit_skip_reasons() {
        let now = Utc::now();
        let codex = ProviderAuthProfile::new(
            "codex",
            "openai-codex",
            "codex_oauth",
            AuthProfileSource::ExternalCodex,
        );
        let api_key =
            ProviderAuthProfile::new("api", "anthropic", "api_key", AuthProfileSource::Env);

        assert_eq!(
            codex.skip_reason_for("anthropic", &["oauth"], now),
            AuthProfileSkipReason::ProviderMismatch
        );
        assert_eq!(
            api_key.skip_reason_for("anthropic", &["oauth"], now),
            AuthProfileSkipReason::AuthKindMismatch
        );
    }

    #[test]
    fn success_clears_cooldown_and_failure_count() {
        let now = Utc::now();
        let mut profile = profile("primary");
        profile.mark_failure(now - Duration::minutes(5));
        profile.start_cooldown(now + Duration::minutes(10), "usage_limited");

        profile.mark_success(now);

        assert_eq!(profile.failure_count, 0);
        assert_eq!(profile.cooldown_until, None);
        assert_eq!(profile.last_good_at, Some(now));
    }
}
