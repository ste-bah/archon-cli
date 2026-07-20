use std::collections::HashMap;

use crate::auth::AuthProvider;

use super::{resolve_betas, version_from_package_json};

/// Beta strings always sent (primary identity + unconditionally required).
pub const DEFAULT_BETAS: &[&str] = &[
    "claude-code-20250219", // primary identity marker -- MUST always be present
    "oauth-2025-04-20",     // required for OAuth auth
    "interleaved-thinking-2025-05-14", // required for thinking blocks
    "prompt-caching-scope-2026-01-05", // required for 1P cache scopes
];

/// Conditional betas -- only sent when their feature is active.
/// These are NOT included by default because the API rejects unknown/inactive betas.
pub const CONDITIONAL_BETAS: &[(&str, &str)] = &[
    ("context-management-2025-06-27", "context_management"),
    ("context-1m-2025-08-07", "context_1m"),
    ("effort-2025-11-24", "effort"),
    ("redact-thinking-2026-02-12", "redact_thinking"),
    ("fast-mode-2026-02-01", "fast_mode"),
    ("structured-outputs-2025-12-15", "structured_outputs"),
    ("task-budgets-2026-03-13", "task_budgets"),
    ("afk-mode-2026-01-31", "afk_mode"),
];

#[derive(Debug, Clone)]
pub enum IdentityMode {
    Spoof {
        version: String,
        entrypoint: String,
        betas: Vec<String>,
        workload: Option<String>,
        anti_distillation: bool,
    },
    Clean,
    Custom {
        user_agent: String,
        x_app: String,
        extra_headers: HashMap<String, String>,
    },
}

#[derive(Debug, Clone, Copy)]
pub struct CustomIdentityConfigView<'a> {
    pub user_agent: &'a str,
    pub x_app: &'a str,
    pub extra_headers: Option<&'a HashMap<String, String>>,
}

#[derive(Debug, Clone, Copy)]
pub struct IdentityConfigView<'a> {
    pub mode: &'a str,
    pub spoof_version: &'a str,
    pub spoof_entrypoint: &'a str,
    pub spoof_betas: Option<&'a [String]>,
    pub anti_distillation: bool,
    pub workload: Option<&'a str>,
    pub custom: Option<CustomIdentityConfigView<'a>>,
}

impl Default for IdentityConfigView<'static> {
    fn default() -> Self {
        Self {
            mode: "clean",
            spoof_version: "2.1.89",
            spoof_entrypoint: "cli",
            spoof_betas: None,
            anti_distillation: false,
            workload: None,
            custom: None,
        }
    }
}

/// Resolve Anthropic identity mode from auth plus user configuration.
///
/// OAuth-shaped Anthropic credentials must always use the Claude Code identity
/// envelope. The Messages API rejects those tokens when they are sent as a
/// normal third-party API client, so auth kind has higher precedence than
/// config or CLI identity flags. Codex OAuth is intentionally excluded because
/// it targets the OpenAI Responses API, not Anthropic Messages.
pub fn resolve_identity_mode(
    auth: &AuthProvider,
    force_spoof: bool,
    config: &IdentityConfigView<'_>,
) -> IdentityMode {
    if matches!(
        auth,
        AuthProvider::OAuthToken(_) | AuthProvider::BearerToken(_)
    ) || force_spoof
        || config.mode == "spoof"
    {
        return spoof_identity_mode(config);
    }

    if config.mode == "custom" {
        let custom = config.custom;
        return IdentityMode::Custom {
            user_agent: custom
                .map(|c| c.user_agent.to_string())
                .unwrap_or_else(|| concat!("archon-cli/", env!("CARGO_PKG_VERSION")).into()),
            x_app: custom
                .map(|c| c.x_app.to_string())
                .unwrap_or_else(|| "archon".into()),
            extra_headers: custom
                .and_then(|c| c.extra_headers)
                .cloned()
                .unwrap_or_default(),
        };
    }

    IdentityMode::Clean
}

fn spoof_identity_mode(config: &IdentityConfigView<'_>) -> IdentityMode {
    // Spoof version priority (was inverted — fixed 2026-05-12):
    //   1. Operator config (`identity.spoof_version` in config.toml) wins.
    //      This makes tests deterministic and respects explicit operator
    //      intent. archon trusts what the user configured.
    //   2. Fall back to whatever `version_from_package_json()` finds (the
    //      locally-installed claude-cli's package.json version) only when
    //      the operator left `spoof_version` empty. This preserves the
    //      auto-tracking defense for users who never touch the config but
    //      have a local claude-cli install.
    //   3. If both are absent, the empty string propagates and downstream
    //      header construction will produce `claude-cli/ (external, cli)`
    //      — visible enough to catch in testing, not a silent crash.
    let configured = config.spoof_version.trim();
    let version = if !configured.is_empty() {
        configured.to_string()
    } else {
        version_from_package_json().unwrap_or_default()
    };
    IdentityMode::Spoof {
        version,
        entrypoint: config.spoof_entrypoint.to_string(),
        betas: resolve_betas(config.spoof_betas),
        workload: config.workload.map(str::to_string),
        anti_distillation: config.anti_distillation,
    }
}
