//! `archon/config` — reading and writing the session knobs the IDE can reach
//! (issue #26, item 4).
//!
//! Deliberately a short, closed list. The previous implementation answered
//! `{"value": null}` for every key, which is indistinguishable from "that key
//! exists and is unset" — so a typo in an extension setting looked like a
//! working read. An unknown key is now an error, and a write to a read-only
//! key is an error, because both are client bugs worth surfacing.

use std::sync::Arc;

use tokio::sync::Mutex;

use crate::ide::runtime::IdeAgentRuntime;

/// Model in use for this session. Read-only: switching models mid-session is
/// `/model`'s job and has conversation-state consequences the IDE does not
/// currently handle.
pub const KEY_MODEL: &str = "model";
/// Live permission mode. Read/write — this is the knob that decides whether
/// the agent asks before running a tool.
pub const KEY_PERMISSION_MODE: &str = "permissionMode";
/// Directory the agent is working in. Read-only: it is the process cwd, set
/// by `--workspace` at spawn.
pub const KEY_WORKSPACE: &str = "workspace";

/// Every key `archon/config` understands.
pub const KNOWN_KEYS: &[&str] = &[KEY_MODEL, KEY_PERMISSION_MODE, KEY_WORKSPACE];

/// Read `key`, or say why it cannot be read.
pub fn read(runtime: Option<&IdeAgentRuntime>, key: &str) -> Result<serde_json::Value, String> {
    match key {
        KEY_WORKSPACE => Ok(serde_json::json!(
            std::env::current_dir()
                .map(|dir| dir.display().to_string())
                .map_err(|error| format!("working directory is not readable: {error}"))?
        )),
        KEY_MODEL => {
            let runtime = require_runtime(runtime, key)?;
            Ok(serde_json::json!(runtime.model()))
        }
        KEY_PERMISSION_MODE => {
            let runtime = require_runtime(runtime, key)?;
            Ok(serde_json::json!(lock_mode(&runtime.permission_mode())?))
        }
        other => Err(unknown_key(other)),
    }
}

/// Write `value` to `key`, or say why it cannot be written.
pub fn write(
    runtime: Option<&IdeAgentRuntime>,
    key: &str,
    value: &serde_json::Value,
) -> Result<(), String> {
    match key {
        KEY_MODEL | KEY_WORKSPACE => {
            Err(format!("config key '{key}' is read-only for IDE sessions"))
        }
        KEY_PERMISSION_MODE => {
            let mode = value
                .as_str()
                .ok_or_else(|| format!("config key '{key}' takes a string, got {value}"))?;
            // Validated with the gate's own parser, so an unrecognised mode
            // is refused here rather than silently falling back to the
            // default the gate would pick for it.
            mode.parse::<archon_permissions::mode::PermissionMode>()
                .map_err(|error| format!("'{mode}' is not a permission mode: {error}"))?;
            let runtime = require_runtime(runtime, key)?;
            let handle = runtime.permission_mode();
            let mut slot = handle
                .try_lock()
                .map_err(|_| format!("config key '{key}' is busy; retry"))?;
            mode.clone_into(&mut slot);
            Ok(())
        }
        other => Err(unknown_key(other)),
    }
}

fn require_runtime<'a>(
    runtime: Option<&'a IdeAgentRuntime>,
    key: &str,
) -> Result<&'a IdeAgentRuntime, String> {
    runtime.ok_or_else(|| format!("config key '{key}' needs an agent, and none is attached"))
}

/// Read the shared permission mode without blocking the dispatcher.
///
/// The agent takes this lock briefly on every tool call, so contention is
/// possible; reporting the contention beats blocking the JSON-RPC loop or
/// inventing a value.
fn lock_mode(handle: &Arc<Mutex<String>>) -> Result<String, String> {
    handle
        .try_lock()
        .map(|mode| mode.clone())
        .map_err(|_| "permission mode is busy; retry".to_string())
}

fn unknown_key(key: &str) -> String {
    format!(
        "unknown config key '{key}'; known keys are {}",
        KNOWN_KEYS.join(", ")
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn workspace_reads_without_an_agent() {
        let value = read(None, KEY_WORKSPACE).expect("workspace is process state");

        assert!(value.as_str().is_some_and(|dir| !dir.is_empty()));
    }

    /// The bug this replaces: every key answered `null`, so a misspelt key
    /// looked exactly like a real one that happened to be unset.
    #[test]
    fn an_unknown_key_is_an_error_rather_than_a_null() {
        let error = read(None, "permisionMode").expect_err("typo must not read as unset");

        assert!(error.contains("unknown config key"), "{error}");
        assert!(error.contains(KEY_PERMISSION_MODE), "{error}");
    }

    #[test]
    fn a_read_only_key_refuses_writes() {
        let error = write(None, KEY_MODEL, &serde_json::json!("some-model"))
            .expect_err("model is read-only");

        assert!(error.contains("read-only"), "{error}");
    }

    #[test]
    fn a_key_needing_the_agent_says_so_when_none_is_attached() {
        let error = read(None, KEY_MODEL).expect_err("no agent, no model");

        assert!(error.contains("no agent is attached") || error.contains("none is attached"));
    }

    #[test]
    fn a_bogus_permission_mode_is_rejected_before_it_reaches_the_gate() {
        let error = write(None, KEY_PERMISSION_MODE, &serde_json::json!("yolo-max"))
            .expect_err("not a mode");

        assert!(error.contains("not a permission mode"), "{error}");
    }
}
