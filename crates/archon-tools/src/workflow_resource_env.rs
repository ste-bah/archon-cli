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

/// The marker file a repository carries when it builds with cargo.
///
/// The variables below name one toolchain, so they are applied only where that
/// toolchain is in use. A workflow implements whatever its PRD describes and the
/// next one may be Go or TypeScript; those repositories must not be handed
/// cargo's variables just because this engine happens to be written in Rust.
const CARGO_MARKER: &str = "Cargo.toml";

pub(crate) fn apply_workflow_resource_defaults(
    env: &mut Vec<(String, String)>,
    command: &str,
    repository_root: &std::path::Path,
    limits: &CargoResourceLimits,
) {
    // Archon's own variable, not a toolchain's — every command gets it.
    ensure_env_default(
        env,
        "ARCHON_WORKFLOW_RESOURCE_CLASS",
        &limits.resource_class,
    );
    if !uses_cargo(command, repository_root) {
        return;
    }
    ensure_env_default(
        env,
        "CARGO_INCREMENTAL",
        if limits.incremental { "1" } else { "0" },
    );
    ensure_env_default(env, "CARGO_BUILD_JOBS", &limits.build_jobs.to_string());
}

/// Whether cargo is in play for this command.
///
/// Asking the repository what it contains, rather than only pattern-matching
/// the command, is what keeps indirect invocations covered — a `make` target, a
/// shell script, a `build.rs` shelling out. That coverage was the reason
/// `CARGO_INCREMENTAL` was previously set for every command in every
/// repository; the marker preserves it without assuming every project is Rust.
/// The command check remains for the case the marker cannot see: cargo run
/// against a manifest somewhere other than the working directory.
fn uses_cargo(command: &str, repository_root: &std::path::Path) -> bool {
    crate::build_cache_env::repository_uses_marker(repository_root, CARGO_MARKER)
        || contains_shell_word(command, "cargo")
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

    /// A directory carrying no marker, so the command stays the only trigger.
    fn non_cargo_repo() -> &'static std::path::Path {
        std::path::Path::new("/nonexistent/not-a-repository")
    }

    #[test]
    fn cargo_commands_get_configured_limits() {
        let mut env = Vec::new();

        apply_workflow_resource_defaults(
            &mut env,
            "cargo test -p demo",
            non_cargo_repo(),
            &limits(),
        );

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

        apply_workflow_resource_defaults(&mut env, "cargo build", non_cargo_repo(), &limits);

        assert!(env.contains(&("CARGO_BUILD_JOBS".to_string(), "11".to_string())));
    }

    #[test]
    fn incremental_can_be_turned_on() {
        let mut env = Vec::new();
        let limits = CargoResourceLimits {
            incremental: true,
            ..limits()
        };

        apply_workflow_resource_defaults(&mut env, "cargo build", non_cargo_repo(), &limits);

        assert!(env.contains(&("CARGO_INCREMENTAL".to_string(), "1".to_string())));
    }

    #[test]
    fn resource_class_follows_config() {
        let mut env = Vec::new();
        let limits = CargoResourceLimits {
            resource_class: "full".into(),
            ..limits()
        };

        apply_workflow_resource_defaults(&mut env, "npm test", non_cargo_repo(), &limits);

        assert!(env.contains(&(
            "ARCHON_WORKFLOW_RESOURCE_CLASS".to_string(),
            "full".to_string()
        )));
    }

    /// A project that is not Rust gets no cargo variables at all.
    ///
    /// `CARGO_INCREMENTAL` used to be set on every command in every repository.
    /// Harmless to a Node or Go build, which ignores a name it does not know,
    /// but it is one toolchain's variable being asserted over projects that do
    /// not use it, and the engine is not entitled to assume its own language.
    #[test]
    fn non_cargo_commands_get_only_generic_resource_class() {
        let mut env = Vec::new();

        apply_workflow_resource_defaults(&mut env, "npm test", non_cargo_repo(), &limits());

        assert!(
            env.iter()
                .any(|(key, _)| key == "ARCHON_WORKFLOW_RESOURCE_CLASS")
        );
        assert!(!env.iter().any(|(key, _)| key == "CARGO_BUILD_JOBS"));
        assert!(!env.iter().any(|(key, _)| key == "CARGO_INCREMENTAL"));
    }

    /// The reason the marker exists rather than only a command check: a build
    /// invoked through something else still gets its limits. `make test` names
    /// no toolchain, and the repository is the only evidence available.
    #[test]
    fn a_cargo_repository_gets_limits_for_an_indirect_invocation() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(dir.path().join("Cargo.toml"), "[package]\n").expect("marker");
        let mut env = Vec::new();

        apply_workflow_resource_defaults(&mut env, "make test", dir.path(), &limits());

        assert!(env.contains(&("CARGO_BUILD_JOBS".to_string(), "6".to_string())));
        assert!(env.contains(&("CARGO_INCREMENTAL".to_string(), "0".to_string())));
    }

    /// And a non-Rust repository stays clean even when a build tool is invoked
    /// the same indirect way.
    #[test]
    fn a_non_cargo_repository_stays_clean_for_an_indirect_invocation() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(dir.path().join("package.json"), "{}\n").expect("marker");
        let mut env = Vec::new();

        apply_workflow_resource_defaults(&mut env, "make test", dir.path(), &limits());

        assert!(!env.iter().any(|(key, _)| key.starts_with("CARGO_")));
    }

    #[test]
    fn explicit_env_values_are_preserved() {
        let mut env = vec![("CARGO_BUILD_JOBS".to_string(), "4".to_string())];

        apply_workflow_resource_defaults(&mut env, "cargo test", non_cargo_repo(), &limits());

        assert!(env.contains(&("CARGO_BUILD_JOBS".to_string(), "4".to_string())));
        assert!(!env.contains(&("CARGO_BUILD_JOBS".to_string(), "6".to_string())));
    }

    #[test]
    fn shell_word_detection_ignores_substrings() {
        let mut env = Vec::new();

        apply_workflow_resource_defaults(&mut env, "echo xcargo", non_cargo_repo(), &limits());

        assert!(!env.iter().any(|(key, _)| key == "CARGO_BUILD_JOBS"));
    }

    /// Auto must never emit `0`, which cargo reads as an error rather than as
    /// "pick for me".
    #[test]
    fn auto_build_jobs_is_at_least_one() {
        assert!(default_build_jobs() >= 1);
    }
}
