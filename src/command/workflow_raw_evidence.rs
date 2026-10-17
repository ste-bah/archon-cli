//! Run-local index into durable subagent transcripts, plus raw author artifacts.
use archon_workflow::{
    WorkflowAgentOutcome, WorkflowError, WorkflowResult, WorkflowV2AgentError, WorkflowV2Evidence,
    WorkflowV2EvidenceKind, WorkflowV2Result,
};
use serde_json::{Value, json};
use std::{
    io::Write,
    path::{Path, PathBuf},
};

pub(super) struct RawEvidence {
    path: PathBuf,
    record: Value,
    finished: bool,
}
impl RawEvidence {
    pub(super) fn start(v2: &Path, call: &str, prompt: &str) -> WorkflowResult<Self> {
        let root = v2
            .parent()
            .ok_or_else(|| WorkflowError::StageFailed("missing run root".into()))?;
        let id = call
            .chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                    c
                } else {
                    '_'
                }
            })
            .collect::<String>();
        let transcript = dirs::home_dir().map(|home| {
            home.join(".archon/sessions")
                .join(root.file_name().unwrap())
                .join("subagents")
        });
        let record = json!({"call_id":call,"status":"running", "transcript_directory":transcript,
            "transcript_note":"Messages and tool inputs/results are flushed after each round in this directory; filenames include the stage call id."});
        save(
            &root.join("prompts").join(format!("{id}.json")),
            &json!({"call_id":call,"prompt":prompt}),
        )?;
        let value = Self {
            path: root.join("agent-outputs").join(format!("{id}.json")),
            record,
            finished: false,
        };
        save(&value.path, &value.record)?;
        Ok(value)
    }
    pub(super) fn finish(
        &mut self,
        result: &Result<WorkflowAgentOutcome, WorkflowV2AgentError>,
    ) -> WorkflowResult<()> {
        match result {
            Ok(outcome) => {
                self.record["status"] = json!("completed");
                self.record["content"] = json!(outcome.content);
                self.record["stop_reason"] = json!(outcome.stop_reason);
                self.record["tool_uses"] = json!(
                    outcome
                        .tool_uses
                        .iter()
                        .map(|t| {
                            // Issue 276: what a call touched, never its content.
                            let input = archon_tools::tool_trace_input::safe_input(
                                &t.tool_name,
                                &t.input,
                                384,
                            );
                            json!({"name":t.tool_name,"input":input.input,"output":t.output})
                        })
                        .collect::<Vec<_>>()
                );
            }
            Err(error) => {
                self.record["status"] = json!("failed");
                self.record["error"] = json!(error.to_string());
            }
        }
        save(&self.path, &self.record)?;
        self.finished = true;
        Ok(())
    }
}
impl Drop for RawEvidence {
    fn drop(&mut self) {
        if !self.finished {
            self.record["status"] = json!("interrupted");
            if let Err(error) = save(&self.path, &self.record) {
                tracing::error!(%error, "raw author evidence write failed");
            }
        }
    }
}
/// The stored result of a raw provider outcome. It carries no agent report,
/// so what the session read and ran comes from its tool trace (Issue 276).
pub(super) fn raw_outcome_result(
    outcome: WorkflowAgentOutcome,
    stop_reason: String,
) -> WorkflowV2Result {
    let mut result = WorkflowV2Result::accepted("trusted raw provider outcome captured");
    result.evidence.push(WorkflowV2Evidence::new(
        WorkflowV2EvidenceKind::Inspection,
        "fixed decomposition author returned provider content and typed stop reason",
    ));
    result.data = json!({
        "content": outcome.content,
        "stopReason": stop_reason,
        "tokensIn": outcome.tokens_in,
        "tokensInUnknownReason": outcome.tokens_in.is_none().then_some("provider did not report a confirmed input token count"),
        "tokensOut": outcome.tokens_out,
        "tokensOutUnknownReason": outcome.tokens_out.is_none().then_some("provider did not report an output token count"),
    });
    archon_workflow::v2::tool_trace::record_tool_trace(&mut result, &outcome.tool_uses);
    result
}

fn save(path: &Path, value: &Value) -> WorkflowResult<()> {
    let write = || -> std::io::Result<()> {
        std::fs::create_dir_all(path.parent().unwrap())?;
        let temporary = path.with_extension(format!("{}.tmp", uuid::Uuid::new_v4()));
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(&temporary)?;
        let value = archon_workflow::events::sanitize_value(value.clone());
        let serialized = value.to_string();
        let redacted = archon_observability::shared_redactor::redact_text(
            &serialized,
            archon_workflow::events::REDACTION_MARKER,
        );
        let output = if serde_json::from_str::<Value>(&redacted).is_ok() {
            redacted
        } else {
            tracing::warn!(
                path = %path.display(),
                reason = "invalid JSON after the second pass",
                "raw evidence fallback redaction applied"
            );
            redact_structured_service_accounts(value, archon_workflow::events::REDACTION_MARKER)
                .to_string()
        };
        file.write_all(output.as_bytes())?;
        file.sync_data()?;
        std::fs::rename(temporary, path)
    };
    write()
        .map_err(|e| WorkflowError::StageFailed(format!("author evidence {}: {e}", path.display())))
}

fn redact_structured_service_accounts(value: Value, marker: &str) -> Value {
    if let Value::Object(map) = &value
        && map.iter().any(|(key, value)| {
            key.eq_ignore_ascii_case("type")
                && value
                    .as_str()
                    .is_some_and(|kind| kind.eq_ignore_ascii_case("service_account"))
        })
    {
        return Value::String(marker.to_string());
    }
    match value {
        Value::Object(map) => Value::Object(
            map.into_iter()
                .map(|(key, value)| (key, redact_structured_service_accounts(value, marker)))
                .collect(),
        ),
        Value::Array(values) => Value::Array(
            values
                .into_iter()
                .map(|value| redact_structured_service_accounts(value, marker))
                .collect(),
        ),
        other => other,
    }
}

#[cfg(test)]
#[path = "workflow_raw_evidence_tests.rs"]
mod tests;
