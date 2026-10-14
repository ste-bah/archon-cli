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
            tokens_in: Some(0),
            tokens_out: Some(0),
            stop_reason: Some("end_turn".into()),
        };
        evidence.finish(&Ok(outcome)).unwrap();
        let raw =
            std::fs::read_to_string(temp.path().join("run/agent-outputs/author-1.json")).unwrap();
        assert!(!raw.contains("sk-ant-"), "{raw}");
        assert!(!raw.contains("opaque-bearer-credential-123"), "{raw}");
    }

    #[test]
    fn raw_outcome_omits_unreported_usage_and_keeps_reported_counts() {
        let absent = raw_outcome_result(
            WorkflowAgentOutcome {
                tokens_in: None,
                tokens_out: None,
                ..Default::default()
            },
            "end_turn".into(),
        );
        assert!(absent.data["tokensIn"].is_null());
        assert_eq!(
            absent.data["tokensInUnknownReason"],
            json!("provider did not report a confirmed input token count")
        );
        assert!(absent.data["tokensOut"].is_null());
        assert_eq!(
            absent.data["tokensOutUnknownReason"],
            json!("provider did not report an output token count")
        );

        let reported = raw_outcome_result(
            WorkflowAgentOutcome {
                tokens_in: Some(123),
                tokens_out: Some(45),
                ..Default::default()
            },
            "end_turn".into(),
        );
        assert_eq!(reported.data["tokensIn"], json!(123));
        assert!(reported.data["tokensInUnknownReason"].is_null());
        assert_eq!(reported.data["tokensOut"], json!(45));
        assert!(reported.data["tokensOutUnknownReason"].is_null());

        let reported_zero = raw_outcome_result(
            WorkflowAgentOutcome {
                tokens_in: Some(0),
                tokens_out: Some(0),
                ..Default::default()
            },
            "end_turn".into(),
        );
        assert_eq!(reported_zero.data["tokensIn"], json!(0));
        assert!(reported_zero.data["tokensInUnknownReason"].is_null());
        assert_eq!(reported_zero.data["tokensOut"], json!(0));
        assert!(reported_zero.data["tokensOutUnknownReason"].is_null());
        let read_back = |result: &WorkflowV2Result| {
            serde_json::from_slice::<Value>(&serde_json::to_vec(result).unwrap()).unwrap()
        };
        let absent_read_back = read_back(&absent);
        let reported_read_back = read_back(&reported);
        let reported_zero_read_back = read_back(&reported_zero);
        assert_eq!(
            absent_read_back["data"]["tokensInUnknownReason"],
            json!("provider did not report a confirmed input token count")
        );
        assert_eq!(reported_read_back["data"]["tokensIn"], json!(123));
        assert_eq!(reported_zero_read_back["data"]["tokensIn"], json!(0));
        println!(
            "EVIDENCE #369 rawOutcome-usage BEFORE: absent input serialized as null without explanation; AFTER separate JSON read: {}",
            absent_read_back["data"]
        );
        println!(
            "EVIDENCE #369 rawOutcome-reported-zero BEFORE: Some(0) may be a measured count; AFTER separate JSON read: {}",
            reported_zero_read_back["data"]
        );
        println!(
            "EVIDENCE #369 rawOutcome-positive-count BEFORE: Some(123) had no availability marker; AFTER separate JSON read: {}",
            reported_read_back["data"]
        );
    }
}
