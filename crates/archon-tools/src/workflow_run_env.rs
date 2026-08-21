//! The run's own identity, handed to every command an agent runs.
//!
//! # The failure this removes
//!
//! A task can declare `required_env_keys`, and a verification branch fails
//! outright when a declared key is absent. One reference task declares a
//! per-review run identifier — a value that is *the workflow run's own id* and
//! cannot be known before the run exists. Nothing set it, so the run reached
//! its terminal task and failed on a value the run was itself holding.
//!
//! Asking the operator to export it by hand before each run is not a fix: the
//! id changes every run, so a hand-set value is stale the moment it is set.
//!
//! # Why a config alias list rather than a known key name
//!
//! The key's *name* is a property of the project's task set, not of the
//! workflow engine, and naming one project's key in engine code is exactly the
//! hardcoding this codebase forbids. So the engine always exports its own
//! canonical name, and a project maps that value onto whatever its tasks
//! actually declare, in `[tools] run_id_env_keys`.
//!
//! # Why here
//!
//! `ToolContext::session_id` already *is* the run id inside a workflow run —
//! `bash_observability` relies on the same `wf-` convention to tag activity
//! events. So the value needs no new plumbing; it only needs to reach the
//! environment of the commands the agent runs.

use crate::bash::bash_env::ensure_env_default;

/// The canonical name, always exported inside a workflow run.
pub const RUN_ID_ENV: &str = "ARCHON_WORKFLOW_RUN_ID";

/// A session id identifies a workflow run when it carries the run prefix.
///
/// Matches `bash_observability::start_bash_heartbeat`, which decides the same
/// question the same way; a session that is not a workflow run has no run id to
/// export and must not get an empty or misleading one.
fn run_id_of(session_id: &str) -> Option<&str> {
    session_id.starts_with("wf-").then_some(session_id)
}

/// Export the run id under the canonical name and under every project alias.
///
/// Applied as a *default*: a value already in the environment wins, so an
/// operator can still override one deliberately for a single command.
/// Aliases that are blank, or that collide with the canonical name, are
/// skipped rather than producing a duplicate entry.
pub(crate) fn apply_workflow_run_identity(
    env: &mut Vec<(String, String)>,
    session_id: &str,
    alias_keys: &[String],
) {
    let Some(run_id) = run_id_of(session_id) else {
        return;
    };
    ensure_env_default(env, RUN_ID_ENV, run_id);
    for key in alias_keys {
        let key = key.trim();
        if key.is_empty() || key == RUN_ID_ENV {
            continue;
        }
        ensure_env_default(env, key, run_id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn value_of<'a>(env: &'a [(String, String)], key: &str) -> Option<&'a str> {
        env.iter()
            .find(|(name, _)| name == key)
            .map(|(_, value)| value.as_str())
    }

    #[test]
    fn a_workflow_session_exports_its_own_run_id() {
        let mut env = Vec::new();
        apply_workflow_run_identity(&mut env, "wf-9d0b7ff3", &[]);
        assert_eq!(value_of(&env, RUN_ID_ENV), Some("wf-9d0b7ff3"));
    }

    /// The whole point: a project's own key name gets the same value without
    /// that name appearing anywhere in engine code.
    #[test]
    fn a_project_alias_receives_the_same_value() {
        let mut env = Vec::new();
        let aliases = vec!["PROJECT_REVIEW_RUN_ID".to_string()];
        apply_workflow_run_identity(&mut env, "wf-9d0b7ff3", &aliases);
        assert_eq!(value_of(&env, "PROJECT_REVIEW_RUN_ID"), Some("wf-9d0b7ff3"));
    }

    /// An ordinary interactive session has no run id, and must not be given a
    /// misleading one.
    #[test]
    fn a_non_workflow_session_exports_nothing() {
        let mut env = Vec::new();
        apply_workflow_run_identity(&mut env, "sess-1234", &["ANY_KEY".to_string()]);
        assert!(env.is_empty(), "got: {env:?}");
    }

    #[test]
    fn an_explicit_value_already_in_the_environment_wins() {
        let mut env = vec![(RUN_ID_ENV.to_string(), "wf-pinned".to_string())];
        apply_workflow_run_identity(&mut env, "wf-9d0b7ff3", &[]);
        assert_eq!(value_of(&env, RUN_ID_ENV), Some("wf-pinned"));
    }

    #[test]
    fn blank_and_self_referential_aliases_are_skipped() {
        let mut env = Vec::new();
        let aliases = vec![
            "  ".to_string(),
            RUN_ID_ENV.to_string(),
            " SPACED_KEY ".to_string(),
        ];
        apply_workflow_run_identity(&mut env, "wf-9d0b7ff3", &aliases);
        assert_eq!(env.len(), 2, "canonical plus one trimmed alias: {env:?}");
        assert_eq!(value_of(&env, "SPACED_KEY"), Some("wf-9d0b7ff3"));
    }
}
