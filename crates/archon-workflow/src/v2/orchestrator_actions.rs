//! Orchestrated lifecycle (v3) — typed actions the orchestrator model emits.
//!
//! The orchestrator reconstructs each turn from the authoritative task universe,
//! deterministic orchestration ledger, and a bounded recent transcript tail. It
//! spawns focused subagents, reads their final reports and the host's typed gate
//! verdicts verbatim, and decides routing from that bounded state. The host
//! enforces; the orchestrator decides. Malformed actions are returned as typed
//! repair feedback on the next bounded turn.

use serde::{Deserialize, Serialize};

/// One action per orchestrator turn. The model replies with exactly one JSON
/// object matching this enum (tagged by `action`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum OrchestratorAction {
    /// Spawn a coder subagent in a sealed worktree through the write gauntlet.
    SpawnCoder {
        task_id: String,
        /// Full instructions for the coder; include acceptance criteria and,
        /// on retries, the verbatim failure output from the prior attempt.
        instructions: String,
        target_files: Vec<String>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        focused_tests: Vec<String>,
    },
    /// Spawn a read-only verifier subagent; the host independently re-runs
    /// declared deliverable-contract commands beside it.
    SpawnVerifier {
        task_id: String,
        instructions: String,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        checks: Vec<String>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        artifact_paths: Vec<String>,
    },
    /// Spawn a bounded read-only explorer for discovery questions.
    SpawnExplorer { question: String },
    /// Request completion credit. The host re-validates every gate before
    /// granting it and refuses with a typed error into the conversation
    /// when any check fails.
    AcceptTask {
        task_id: String,
        evidence_summary: String,
    },
    /// Record an honest block with the accumulated evidence.
    BlockTask { task_id: String, reason: String },
    /// End the run; the host assembles the typed report and accounting.
    FinalReport { narrative: String },
}

/// Typed result of dispatching one action, fed back verbatim into the
/// orchestrator conversation as the next user turn.
#[derive(Debug, Clone, Serialize)]
pub struct ActionOutcome {
    pub action_ordinal: usize,
    pub tool: String,
    /// "ok" | "gate_rejected" | "refused" | "error"
    pub status: String,
    /// Subagent final report and/or host gate verdict, verbatim.
    pub report: serde_json::Value,
}

/// Extract the action from an orchestrator reply envelope. Accepts the action
/// under `data.action`, at the top level, or embedded in any string field —
/// tolerance only ever locates the object; it never reshapes semantics.
pub(crate) fn action_from_reply(reply: &serde_json::Value) -> Result<OrchestratorAction, String> {
    for pointer in ["/data/action", "/action", "/result/data/action"] {
        if let Some(value) = reply.pointer(pointer) {
            return serde_json::from_value(value.clone()).map_err(|err| {
                format!("the object at {pointer} is not a valid action ({err}); reply with a valid action envelope")
            });
        }
    }
    let mut texts = Vec::new();
    collect_strings(reply, &mut texts);
    for text in &texts {
        if text.contains("\"action\"")
            && let Ok(action) = parse_action(text)
        {
            return Ok(action);
        }
    }
    Err(
        "no action found in your reply; reply with exactly one JSON envelope {\"status\":\"accepted\",\"summary\":\"...\",\"data\":{\"action\":{...}}}"
            .to_string(),
    )
}

fn collect_strings(value: &serde_json::Value, output: &mut Vec<String>) {
    match value {
        serde_json::Value::String(text) => output.push(text.clone()),
        serde_json::Value::Array(items) => {
            for item in items {
                collect_strings(item, output);
            }
        }
        serde_json::Value::Object(fields) => {
            for nested in fields.values() {
                collect_strings(nested, output);
            }
        }
        _ => {}
    }
}

pub(crate) fn parse_action(body: &str) -> Result<OrchestratorAction, String> {
    let trimmed = body.trim();
    let candidate = extract_json_object(trimmed).unwrap_or(trimmed);
    serde_json::from_str::<OrchestratorAction>(candidate).map_err(|err| {
        format!(
            "your reply was not a single valid action object ({err}); reply with exactly one JSON object matching the documented action schema"
        )
    })
}

/// Tolerate prose around the object (e.g. a fenced block) without ever
/// reshaping semantics: find the outermost balanced object.
fn extract_json_object(body: &str) -> Option<&str> {
    let start = body.find('{')?;
    let mut depth = 0usize;
    let mut in_string = false;
    let mut escaped = false;
    for (offset, ch) in body[start..].char_indices() {
        match ch {
            _ if escaped => escaped = false,
            '\\' if in_string => escaped = true,
            '"' => in_string = !in_string,
            '{' if !in_string => depth += 1,
            '}' if !in_string => {
                depth -= 1;
                if depth == 0 {
                    return Some(&body[start..start + offset + ch.len_utf8()]);
                }
            }
            _ => {}
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_tagged_action_with_surrounding_prose() {
        let body = r#"I will retry with the corrected filter.
```json
{"action":"spawn_coder","task_id":"TASK-EX-001","instructions":"run cargo test module::path::test_name","target_files":["src/lib.rs"]}
```"#;
        let action = parse_action(body).expect("action");
        assert!(matches!(
            action,
            OrchestratorAction::SpawnCoder { ref task_id, .. } if task_id == "TASK-EX-001"
        ));
    }

    #[test]
    fn malformed_action_returns_correctable_error() {
        let err = parse_action("{\"action\":\"unknown_thing\"}").expect_err("must fail");
        assert!(err.contains("action schema"));
    }

    #[test]
    fn final_report_and_block_round_trip() {
        for body in [
            r#"{"action":"final_report","narrative":"done"}"#,
            r#"{"action":"block_task","task_id":"TASK-EX-002","reason":"provider entitlement missing"}"#,
        ] {
            parse_action(body).expect("valid action");
        }
    }
}
