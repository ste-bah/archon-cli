//! Run-local index into durable subagent transcripts, plus raw author artifacts.
use archon_workflow::{WorkflowAgentOutcome, WorkflowError, WorkflowResult, WorkflowV2AgentError};
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
                        .map(|t| json!({"name":t.tool_name,"input":t.input,"output":t.output}))
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
        file.write_all(
            archon_observability::redaction::redact_text(&value.to_string()).as_bytes(),
        )?;
        file.sync_data()?;
        std::fs::rename(temporary, path)
    };
    write()
        .map_err(|e| WorkflowError::StageFailed(format!("author evidence {}: {e}", path.display())))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Issue-245: the agent-output record is a public copy and stays redacted.
    #[test]
    fn the_public_agent_output_record_redacts_real_looking_secrets() {
        let temp = tempfile::tempdir().unwrap();
        let v2 = temp.path().join("run").join("v2");
        let mut evidence = RawEvidence::start(&v2, "author-1", "prompt").unwrap();
        let outcome = WorkflowAgentOutcome {
            content: "key sk-ant-api03-abcdefghijklmnopqrstuvwxyz0123456789 and Authorization: Bearer opaque-bearer-credential-123".into(),
            tool_uses: Vec::new(),
            tokens_in: 0,
            tokens_out: 0,
            stop_reason: Some("end_turn".into()),
        };
        evidence.finish(&Ok(outcome)).unwrap();
        let raw =
            std::fs::read_to_string(temp.path().join("run/agent-outputs/author-1.json")).unwrap();
        assert!(!raw.contains("sk-ant-"), "{raw}");
        assert!(!raw.contains("opaque-bearer-credential-123"), "{raw}");
    }
}
