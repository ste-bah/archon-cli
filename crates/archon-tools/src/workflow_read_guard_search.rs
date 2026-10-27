use super::{AgentKey, WorkflowReadGuard, read_only, result_fits_replay};
use crate::tool::ToolContext;
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

#[derive(Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(super) struct SearchKey {
    pub(super) session: AgentKey,
    path: PathBuf,
    pattern: String,
    glob: Option<String>,
    mode: String,
    case_insensitive: bool,
}

impl WorkflowReadGuard {
    /// Compare the current rendered result, after the search really ran.
    pub(crate) fn search_result(
        &self,
        ctx: &ToolContext,
        path: &Path,
        input: &Value,
        output: &str,
    ) -> Option<String> {
        if !result_fits_replay(
            output,
            ctx.read_result_replay_limit_bytes,
            self.result_note_for(ctx, "Grep", input).as_deref(),
        ) {
            return None;
        }
        let session = (ctx.session_id.clone(), ctx.subagent_id.clone());
        let key = SearchKey {
            session: session.clone(),
            path: path.to_owned(),
            pattern: input["pattern"].as_str().unwrap_or_default().to_owned(),
            glob: input.get("glob").and_then(Value::as_str).map(str::to_owned),
            mode: input["output_mode"]
                .as_str()
                .unwrap_or("files_with_matches")
                .to_owned(),
            case_insensitive: input["case_insensitive"].as_bool().unwrap_or(false),
        };
        let hash = format!("{:x}", Sha256::digest(output.as_bytes()));
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if let Some((old_hash, call)) = state.searches.get(&key)
            && old_hash == &hash
        {
            let call = *call;
            let budget_note = if self.read_only() {
                read_only::refund_inspection(&mut state, &session);
                ""
            } else {
                " This still consumes the read-before-write budget."
            };
            let requested_path = input["path"].as_str().unwrap_or(".");
            return Some(format!(
                "{requested_path}: search result unchanged since your earlier Grep call {call}; content omitted.{budget_note}"
            ));
        }
        let call = state
            .agent_state
            .get(&session)
            .map_or(state.calls, |agent| agent.calls);
        state.searches.insert(key, (hash, call));
        None
    }
}
