//! Resource defaults applied to commands the agent runs through the Bash tool.
//!
//! These used to be three string literals inline: every `cargo` command got
//! `CARGO_BUILD_JOBS=1` with no way to change it short of editing this file. The
//! intent was to stop parallel agents thrashing one machine, but `1` means a
//! single-core build on every host regardless of size, and paired with the Bash
//! timeout it was the likeliest reason long builds were killed rather than
//! finishing slowly.
//!
//! The values now come from `[tools.cargo]` in `config.toml`, carried here as
//! [`CargoResourceLimits`]. This crate cannot name the config type directly —
//! `archon-core` depends on `archon-tools`, so the dependency cannot run the
//! other way — so the caller converts at the construction site, the same way
//! `BashTool::timeout_secs` is already handed over as a plain `u64`.

/// Environment defaults for agent-run `cargo` commands, resolved from
/// `[tools.cargo]`.
///
/// Held by value on `BashTool` rather than looked up per command: the Bash tool
/// is already the thing that owns its limits, and a per-call lookup would need
/// either a global or a config handle threaded through the tool trait.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CargoResourceLimits {
    /// `CARGO_BUILD_JOBS`. Already resolved — `0` never reaches here.
    pub build_jobs: u32,
    /// `CARGO_INCREMENTAL`, as `1` or `0`.
    pub incremental: bool,
    /// `ARCHON_WORKFLOW_RESOURCE_CLASS`.
    pub resource_class: String,
}

impl Default for CargoResourceLimits {
    /// Matches `CargoResourceConfig::default()` in `archon-core`, except that
    /// `build_jobs` is resolved here rather than left as the `0` sentinel: this
    /// type is the post-resolution one, so it carries a usable number.
    fn default() -> Self {
        Self {
            build_jobs: default_build_jobs(),
            incremental: false,
            resource_class: "constrained".into(),
        }
    }
}

/// Half the logical cores, minimum 1 — see `CargoResourceConfig` in
/// `archon-core` for why half rather than all.
fn default_build_jobs() -> u32 {
    let cores = std::thread::available_parallelism()
        .map(std::num::NonZeroUsize::get)
        .unwrap_or(1);
    u32::try_from(cores / 2).unwrap_or(1).max(1)
}

pub(crate) fn apply_workflow_resource_defaults(
    env: &mut Vec<(String, String)>,
    command: &str,
    limits: &CargoResourceLimits,
) {
    ensure_env_default(
        env,
        "ARCHON_WORKFLOW_RESOURCE_CLASS",
        &limits.resource_class,
    );
    // Applied to every command, not just ones with `cargo` as a shell word, so
    // that indirect invocations — a `make` target, a shell script, a build.rs
    // shelling out — are covered too. This matches where `bash_process` used to
    // set it unconditionally; only the value is new.
    ensure_env_default(
        env,
        "CARGO_INCREMENTAL",
        if limits.incremental { "1" } else { "0" },
    );
    if contains_shell_word(command, "cargo") {
        ensure_env_default(env, "CARGO_BUILD_JOBS", &limits.build_jobs.to_string());
    }
}

use crate::bash::bash_env::ensure_env_default;

fn contains_shell_word(command: &str, needle: &str) -> bool {
    command.match_indices(needle).any(|(idx, _)| {
        let before = command[..idx].chars().next_back();
        let after = command[idx + needle.len()..].chars().next();
        !is_word_char(before) && !is_word_char(after)
    })
}

fn is_word_char(ch: Option<char>) -> bool {
    ch.is_some_and(|ch| ch.is_ascii_alphanumeric() || ch == '_' || ch == '-')
}

#[cfg(test)]
mod tests {
    use super::*;

    fn limits() -> CargoResourceLimits {
        CargoResourceLimits {
            build_jobs: 6,
            incremental: false,
            resource_class: "constrained".into(),
        }
    }

    #[test]
    fn cargo_commands_get_configured_limits() {
        let mut env = Vec::new();

        apply_workflow_resource_defaults(&mut env, "cargo test -p demo", &limits());

        assert!(env.contains(&(
            "ARCHON_WORKFLOW_RESOURCE_CLASS".to_string(),
            "constrained".to_string()
        )));
        assert!(env.contains(&("CARGO_BUILD_JOBS".to_string(), "6".to_string())));
        assert!(env.contains(&("CARGO_INCREMENTAL".to_string(), "0".to_string())));
    }

    /// The value that used to be hardcoded is now just one possible setting, so
    /// pin that config actually reaches the environment rather than only that
    /// *some* value does.
    #[test]
    fn build_jobs_follows_config_rather_than_a_constant() {
        let mut env = Vec::new();
        let limits = CargoResourceLimits {
            build_jobs: 11,
            ..limits()
        };

        apply_workflow_resource_defaults(&mut env, "cargo build", &limits);

        assert!(env.contains(&("CARGO_BUILD_JOBS".to_string(), "11".to_string())));
    }

    #[test]
    fn incremental_can_be_turned_on() {
        let mut env = Vec::new();
        let limits = CargoResourceLimits {
            incremental: true,
            ..limits()
        };

        apply_workflow_resource_defaults(&mut env, "cargo build", &limits);

        assert!(env.contains(&("CARGO_INCREMENTAL".to_string(), "1".to_string())));
    }

    #[test]
    fn resource_class_follows_config() {
        let mut env = Vec::new();
        let limits = CargoResourceLimits {
            resource_class: "full".into(),
            ..limits()
        };

        apply_workflow_resource_defaults(&mut env, "npm test", &limits);

        assert!(env.contains(&(
            "ARCHON_WORKFLOW_RESOURCE_CLASS".to_string(),
            "full".to_string()
        )));
    }

    #[test]
    fn non_cargo_commands_get_only_generic_resource_class() {
        let mut env = Vec::new();

        apply_workflow_resource_defaults(&mut env, "npm test", &limits());

        assert!(
            env.iter()
                .any(|(key, _)| key == "ARCHON_WORKFLOW_RESOURCE_CLASS")
        );
        assert!(!env.iter().any(|(key, _)| key == "CARGO_BUILD_JOBS"));
    }

    #[test]
    fn explicit_env_values_are_preserved() {
        let mut env = vec![("CARGO_BUILD_JOBS".to_string(), "4".to_string())];

        apply_workflow_resource_defaults(&mut env, "cargo test", &limits());

        assert!(env.contains(&("CARGO_BUILD_JOBS".to_string(), "4".to_string())));
        assert!(!env.contains(&("CARGO_BUILD_JOBS".to_string(), "6".to_string())));
    }

    #[test]
    fn shell_word_detection_ignores_substrings() {
        let mut env = Vec::new();

        apply_workflow_resource_defaults(&mut env, "echo xcargo", &limits());

        assert!(!env.iter().any(|(key, _)| key == "CARGO_BUILD_JOBS"));
    }

    /// Auto must never emit `0`, which cargo reads as an error rather than as
    /// "pick for me".
    #[test]
    fn auto_build_jobs_is_at_least_one() {
        assert!(default_build_jobs() >= 1);
    }
}
