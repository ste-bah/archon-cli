use std::collections::BTreeMap;
use std::path::PathBuf;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tokio::process::Command;

const DEFAULT_PROFILE_SOURCE: &str = "~/.profile";
const PROFILE_TIMEOUT_MS: u64 = 3_000;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderEnvPolicy {
    #[serde(default)]
    pub required_keys: Vec<String>,
    #[serde(default = "default_profile_sources")]
    pub profile_sources: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

impl ProviderEnvPolicy {
    pub fn new(required_keys: Vec<String>) -> Self {
        Self {
            required_keys: normalize_keys(required_keys),
            profile_sources: default_profile_sources(),
            reason: None,
        }
    }

    pub fn is_empty(&self) -> bool {
        normalize_keys(self.required_keys.clone()).is_empty()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderEnvKeyProof {
    pub key: String,
    pub state: ProviderEnvKeyState,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderEnvKeyState {
    Present,
    Missing,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderEnvCredentialState {
    Present,
    Missing,
    NotRequired,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderEnvProof {
    pub profile_sources_checked: Vec<String>,
    pub redacted_env_keys_checked: Vec<ProviderEnvKeyProof>,
    pub credential_state: ProviderEnvCredentialState,
    #[serde(default)]
    pub errors: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct ProviderEnvResolution {
    pub proof: ProviderEnvProof,
    env: Vec<(String, String)>,
}

impl ProviderEnvResolution {
    pub fn apply_to_env(&self, env: &mut Vec<(String, String)>) {
        for (key, value) in &self.env {
            if let Some(existing) = env.iter_mut().find(|(candidate, _)| candidate == key) {
                existing.1 = value.clone();
            } else {
                env.push((key.clone(), value.clone()));
            }
        }
    }

    pub fn redact_text(&self, text: &str) -> String {
        self.env
            .iter()
            .fold(text.to_string(), |current, (key, value)| {
                if value.is_empty() {
                    current
                } else {
                    current.replace(value, &format!("<redacted:{key}>"))
                }
            })
    }
}

pub async fn resolve_provider_env(policy: &ProviderEnvPolicy) -> ProviderEnvResolution {
    let keys = normalize_keys(policy.required_keys.clone());
    let mut values = env_values(&keys);
    let mut errors = Vec::new();
    let profile_sources = normalize_profiles(&policy.profile_sources);
    if keys.iter().any(|key| !values.contains_key(key)) {
        match profile_values(&keys, &profile_sources).await {
            Ok(profile) => values.extend(profile),
            Err(error) => errors.push(error),
        }
    }
    let proof = proof_for(&keys, &values, profile_sources, errors);
    let env = keys
        .into_iter()
        .filter_map(|key| values.get(&key).map(|value| (key, value.clone())))
        .collect();
    ProviderEnvResolution { proof, env }
}

pub fn provider_env_internal_marker(policy: &ProviderEnvPolicy) -> serde_json::Value {
    serde_json::json!({
        "archon_internal_provider_env_policy": policy,
    })
}

pub fn provider_env_policy_from_marker(value: &serde_json::Value) -> Option<ProviderEnvPolicy> {
    value
        .get("archon_internal_provider_env_policy")
        .and_then(|value| serde_json::from_value(value.clone()).ok())
}

fn proof_for(
    keys: &[String],
    values: &BTreeMap<String, String>,
    profile_sources_checked: Vec<String>,
    errors: Vec<String>,
) -> ProviderEnvProof {
    let redacted_env_keys_checked = keys
        .iter()
        .map(|key| ProviderEnvKeyProof {
            key: key.clone(),
            state: if values.contains_key(key) {
                ProviderEnvKeyState::Present
            } else {
                ProviderEnvKeyState::Missing
            },
        })
        .collect::<Vec<_>>();
    let credential_state = if keys.is_empty() {
        ProviderEnvCredentialState::NotRequired
    } else if redacted_env_keys_checked
        .iter()
        .all(|proof| proof.state == ProviderEnvKeyState::Present)
    {
        ProviderEnvCredentialState::Present
    } else {
        ProviderEnvCredentialState::Missing
    };
    ProviderEnvProof {
        profile_sources_checked,
        redacted_env_keys_checked,
        credential_state,
        errors,
    }
}

fn env_values(keys: &[String]) -> BTreeMap<String, String> {
    keys.iter()
        .filter_map(|key| std::env::var(key).ok().map(|value| (key.clone(), value)))
        .filter(|(_, value)| !value.is_empty())
        .collect()
}

async fn profile_values(
    keys: &[String],
    profiles: &[String],
) -> Result<BTreeMap<String, String>, String> {
    if keys.is_empty() || profiles.is_empty() {
        return Ok(BTreeMap::new());
    }
    let script = profile_script(keys, profiles);
    let output = tokio::time::timeout(
        Duration::from_millis(PROFILE_TIMEOUT_MS),
        Command::new("/bin/zsh").arg("-fc").arg(script).output(),
    )
    .await
    .map_err(|_| "provider env profile preflight timed out".to_string())?
    .map_err(|error| format!("provider env profile preflight failed: {error}"))?;
    Ok(parse_profile_output(&output.stdout))
}

fn profile_script(keys: &[String], profiles: &[String]) -> String {
    let sources = profiles
        .iter()
        .filter_map(|profile| shell_path(profile))
        .map(|profile| {
            format!("if [ -f {profile} ]; then source {profile} >/dev/null 2>&1 || true; fi")
        })
        .collect::<Vec<_>>()
        .join("\n");
    let key_list = keys
        .iter()
        .filter(|key| valid_env_key(key))
        .map(|key| shell_quote(key))
        .collect::<Vec<_>>()
        .join(" ");
    format!(
        "set +x\n{sources}\nfor key in {key_list}; do value=\"${{(P)key-}}\"; printf '%s=%s\\0' \"$key\" \"$value\"; done"
    )
}

fn parse_profile_output(output: &[u8]) -> BTreeMap<String, String> {
    output
        .split(|byte| *byte == 0)
        .filter_map(|chunk| std::str::from_utf8(chunk).ok())
        .filter_map(|entry| entry.split_once('='))
        .filter(|(key, value)| valid_env_key(key) && !value.is_empty())
        .map(|(key, value)| (key.to_string(), value.to_string()))
        .collect()
}

fn normalize_keys(keys: Vec<String>) -> Vec<String> {
    keys.into_iter()
        .map(|key| key.trim().to_ascii_uppercase())
        .filter(|key| valid_env_key(key))
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .collect()
}

fn normalize_profiles(profiles: &[String]) -> Vec<String> {
    let profiles = profiles
        .iter()
        .map(|profile| profile.trim().to_string())
        .filter(|profile| !profile.is_empty())
        .collect::<Vec<_>>();
    if profiles.is_empty() {
        default_profile_sources()
    } else {
        profiles
    }
}

fn shell_path(path: &str) -> Option<String> {
    let expanded = if let Some(rest) = path.strip_prefix("~/") {
        let home = std::env::var("HOME").ok()?;
        PathBuf::from(home).join(rest).display().to_string()
    } else {
        path.to_string()
    };
    Some(shell_quote(&expanded))
}

fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

fn valid_env_key(key: &str) -> bool {
    !key.is_empty()
        && key
            .chars()
            .all(|ch| ch.is_ascii_uppercase() || ch.is_ascii_digit() || ch == '_')
}

fn default_profile_sources() -> Vec<String> {
    vec![DEFAULT_PROFILE_SOURCE.to_string()]
}

#[cfg(test)]
#[path = "provider_env_tests.rs"]
mod tests;
