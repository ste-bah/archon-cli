use std::collections::BTreeMap;
use std::path::PathBuf;
use std::time::{Duration, SystemTime};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
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
    pub found_in: ProviderEnvFoundIn,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderEnvKeyState {
    Present,
    PresentEmpty,
    Missing,
    ResolutionError,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderEnvFoundIn {
    ProcessEnv,
    Profile,
    None,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderEnvProfileProof {
    pub path: String,
    pub exists: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub modified_unix_ms: Option<u128>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content_sha256: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct ProviderEnvResolverProof {
    #[serde(default)]
    pub status: ProviderEnvResolverStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exit_status: Option<i32>,
    #[serde(default)]
    pub stderr: String,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderEnvResolverStatus {
    #[default]
    NotNeeded,
    Succeeded,
    Failed,
    TimedOut,
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
    #[serde(default)]
    pub profile_provenance: Vec<ProviderEnvProfileProof>,
    #[serde(default)]
    pub resolver: ProviderEnvResolverProof,
    pub redacted_env_keys_checked: Vec<ProviderEnvKeyProof>,
    pub credential_state: ProviderEnvCredentialState,
    #[serde(default)]
    pub errors: Vec<String>,
}

#[derive(Clone, PartialEq, Eq)]
pub struct ProviderEnvResolution {
    pub proof: ProviderEnvProof,
    env: Vec<(String, String)>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProviderEnvSource {
    Policy(ProviderEnvPolicy),
    Resolution(ProviderEnvResolution),
    ResolvedPolicy {
        policy: ProviderEnvPolicy,
        resolution: ProviderEnvResolution,
    },
}

impl ProviderEnvSource {
    pub fn policy(&self) -> Option<&ProviderEnvPolicy> {
        match self {
            Self::Policy(policy) | Self::ResolvedPolicy { policy, .. } => Some(policy),
            Self::Resolution(_) => None,
        }
    }

    pub fn resolution(&self) -> Option<&ProviderEnvResolution> {
        match self {
            Self::Resolution(resolution) | Self::ResolvedPolicy { resolution, .. } => {
                Some(resolution)
            }
            Self::Policy(_) => None,
        }
    }
}

impl std::fmt::Debug for ProviderEnvResolution {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ProviderEnvResolution")
            .field("proof", &self.proof)
            .field("resolved_key_count", &self.env.len())
            .finish()
    }
}

impl ProviderEnvResolution {
    pub fn covers(&self, policy: &ProviderEnvPolicy) -> bool {
        let checked = self
            .proof
            .redacted_env_keys_checked
            .iter()
            .map(|proof| proof.key.as_str())
            .collect::<std::collections::BTreeSet<_>>();
        normalize_keys(policy.required_keys.clone())
            .iter()
            .all(|key| checked.contains(key.as_str()))
    }

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
    resolve_provider_env_with_timeout(policy, Duration::from_millis(PROFILE_TIMEOUT_MS)).await
}

async fn resolve_provider_env_with_timeout(
    policy: &ProviderEnvPolicy,
    profile_timeout: Duration,
) -> ProviderEnvResolution {
    let keys = normalize_keys(policy.required_keys.clone());
    let mut values = env_values(&keys);
    let mut origins = values
        .keys()
        .map(|key| (key.clone(), ProviderEnvFoundIn::ProcessEnv))
        .collect::<BTreeMap<_, _>>();
    let mut errors = Vec::new();
    let profile_sources = normalize_profiles(&policy.profile_sources);
    let profile_provenance = profile_provenance(&profile_sources);
    let mut resolver = ProviderEnvResolverProof::default();
    let mut resolution_failed = false;
    if keys.iter().any(|key| !values.contains_key(key)) {
        let attempt = profile_values_with_timeout(&keys, &profile_sources, profile_timeout).await;
        resolver = attempt.resolver;
        if let Some(error) = attempt.error {
            errors.push(error);
            resolution_failed = true;
        } else {
            for (key, value) in attempt.values {
                if let std::collections::btree_map::Entry::Vacant(entry) = values.entry(key) {
                    origins.insert(entry.key().clone(), ProviderEnvFoundIn::Profile);
                    entry.insert(value);
                }
            }
        }
    }
    let proof = proof_for(
        &keys,
        &values,
        &origins,
        ProviderEnvProofInputs {
            profile_sources_checked: profile_sources,
            profile_provenance,
            resolver,
            errors,
            resolution_failed,
        },
    );
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

struct ProviderEnvProofInputs {
    profile_sources_checked: Vec<String>,
    profile_provenance: Vec<ProviderEnvProfileProof>,
    resolver: ProviderEnvResolverProof,
    errors: Vec<String>,
    resolution_failed: bool,
}

fn proof_for(
    keys: &[String],
    values: &BTreeMap<String, String>,
    origins: &BTreeMap<String, ProviderEnvFoundIn>,
    inputs: ProviderEnvProofInputs,
) -> ProviderEnvProof {
    let redacted_env_keys_checked = keys
        .iter()
        .map(|key| ProviderEnvKeyProof {
            key: key.clone(),
            state: match values.get(key) {
                Some(value) if value.is_empty() => ProviderEnvKeyState::PresentEmpty,
                Some(_) => ProviderEnvKeyState::Present,
                None if inputs.resolution_failed => ProviderEnvKeyState::ResolutionError,
                None => ProviderEnvKeyState::Missing,
            },
            found_in: origins
                .get(key)
                .copied()
                .unwrap_or(ProviderEnvFoundIn::None),
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
        profile_sources_checked: inputs.profile_sources_checked,
        profile_provenance: inputs.profile_provenance,
        resolver: inputs.resolver,
        redacted_env_keys_checked,
        credential_state,
        errors: inputs.errors,
    }
}

fn env_values(keys: &[String]) -> BTreeMap<String, String> {
    keys.iter()
        .filter_map(|key| std::env::var(key).ok().map(|value| (key.clone(), value)))
        .collect()
}

struct ProfileAttempt {
    values: BTreeMap<String, String>,
    resolver: ProviderEnvResolverProof,
    error: Option<String>,
}

async fn profile_values_with_timeout(
    keys: &[String],
    profiles: &[String],
    timeout: Duration,
) -> ProfileAttempt {
    if keys.is_empty() || profiles.is_empty() {
        return ProfileAttempt {
            values: BTreeMap::new(),
            resolver: ProviderEnvResolverProof::default(),
            error: None,
        };
    }
    let script = profile_script(keys, profiles);
    let mut command = Command::new(profile_shell());
    command.arg("-c").arg(script).kill_on_drop(true);
    let output = match tokio::time::timeout(timeout, command.output()).await {
        Err(_) => {
            return ProfileAttempt {
                values: BTreeMap::new(),
                resolver: ProviderEnvResolverProof {
                    status: ProviderEnvResolverStatus::TimedOut,
                    ..ProviderEnvResolverProof::default()
                },
                error: Some("provider env profile preflight timed out".to_string()),
            };
        }
        Ok(Err(error)) => {
            return ProfileAttempt {
                values: BTreeMap::new(),
                resolver: ProviderEnvResolverProof {
                    status: ProviderEnvResolverStatus::Failed,
                    ..ProviderEnvResolverProof::default()
                },
                error: Some(format!("provider env profile preflight failed: {error}")),
            };
        }
        Ok(Ok(output)) => output,
    };
    let stderr = if output.stderr.is_empty() {
        String::new()
    } else {
        "<redacted:provider-profile-stderr>".to_string()
    };
    let resolver = ProviderEnvResolverProof {
        status: if output.status.success() {
            ProviderEnvResolverStatus::Succeeded
        } else {
            ProviderEnvResolverStatus::Failed
        },
        exit_status: output.status.code(),
        stderr,
    };
    if !output.status.success() {
        return ProfileAttempt {
            values: BTreeMap::new(),
            resolver,
            error: Some(format!(
                "provider env profile preflight exited with status {}",
                output
                    .status
                    .code()
                    .map_or_else(|| "signal".to_string(), |code| code.to_string())
            )),
        };
    }
    match parse_profile_output(&output.stdout, keys) {
        Ok(values) => ProfileAttempt {
            values,
            resolver,
            error: None,
        },
        Err(error) => ProfileAttempt {
            values: BTreeMap::new(),
            resolver: ProviderEnvResolverProof {
                status: ProviderEnvResolverStatus::Failed,
                ..resolver
            },
            error: Some(error),
        },
    }
}

fn profile_shell() -> PathBuf {
    ["/bin/zsh", "/bin/bash", "/bin/sh"]
        .into_iter()
        .map(PathBuf::from)
        .find(|path| path.is_file())
        .unwrap_or_else(|| PathBuf::from("sh"))
}

fn profile_script(keys: &[String], profiles: &[String]) -> String {
    let sources = profiles
        .iter()
        .filter_map(|profile| shell_path(profile))
        .map(|profile| {
            format!(
                "if [ -f {profile} ]; then source {profile} >/dev/null 2>&1 || {{ print -u2 'provider profile source failed'; exit 73; }}; fi"
            )
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
        "set +x\n{sources}\nfor key in {key_list}; do if env_value=$(printenv \"$key\" 2>/dev/null); then present=1; value=$env_value; else present=0; value=''; fi; printf '%s\\t%s\\t%s\\0' \"$key\" \"$present\" \"$value\"; done"
    )
}

fn parse_profile_output(
    output: &[u8],
    expected_keys: &[String],
) -> Result<BTreeMap<String, String>, String> {
    if !expected_keys.is_empty() && !output.ends_with(&[0]) {
        return Err("provider env profile preflight returned malformed NUL output".to_string());
    }
    let expected = expected_keys
        .iter()
        .map(String::as_str)
        .collect::<std::collections::BTreeSet<_>>();
    let mut seen = std::collections::BTreeSet::new();
    let mut values = BTreeMap::new();
    for chunk in output
        .split(|byte| *byte == 0)
        .filter(|chunk| !chunk.is_empty())
    {
        let entry = std::str::from_utf8(chunk)
            .map_err(|_| "provider env profile preflight returned non-UTF8 output".to_string())?;
        let mut fields = entry.splitn(3, '\t');
        let key = fields.next().unwrap_or_default();
        let present = fields.next().unwrap_or_default();
        let value = fields.next().ok_or_else(|| {
            "provider env profile preflight returned malformed key record".to_string()
        })?;
        if !valid_env_key(key) || !expected.contains(key) || !seen.insert(key) {
            return Err("provider env profile preflight returned invalid key record".to_string());
        }
        match present {
            "1" => {
                values.insert(key.to_string(), value.to_string());
            }
            "0" => {}
            _ => {
                return Err(
                    "provider env profile preflight returned invalid presence state".to_string(),
                );
            }
        }
    }
    if seen.len() != expected.len() {
        return Err("provider env profile preflight omitted key records".to_string());
    }
    Ok(values)
}

fn profile_provenance(profiles: &[String]) -> Vec<ProviderEnvProfileProof> {
    profiles
        .iter()
        .map(|profile| {
            let path = expanded_profile_path(profile);
            let metadata = std::fs::metadata(&path).ok();
            let content_sha256 = std::fs::read(&path).ok().map(|content| {
                let mut hasher = Sha256::new();
                hasher.update(content);
                format!("{:x}", hasher.finalize())
            });
            ProviderEnvProfileProof {
                path: path.display().to_string(),
                exists: metadata.is_some(),
                modified_unix_ms: metadata
                    .and_then(|metadata| metadata.modified().ok())
                    .and_then(|modified| modified.duration_since(SystemTime::UNIX_EPOCH).ok())
                    .map(|duration| duration.as_millis()),
                content_sha256,
            }
        })
        .collect()
}

fn expanded_profile_path(path: &str) -> PathBuf {
    if let Some(rest) = path.strip_prefix("~/")
        && let Ok(home) = std::env::var("HOME")
    {
        return PathBuf::from(home).join(rest);
    }
    PathBuf::from(path)
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
    let expanded = expanded_profile_path(path).display().to_string();
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
