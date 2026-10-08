//! The host session trace on a structured (agent-reported) result, and the
//! check of the agent's claimed reads against it (Issue 276).
use archon_observability::redaction::redact_secret_values;
use archon_tools::subagent_session::TOOL_TRACE_SUMMARY_NAME;
use archon_tools::tool_trace_input::clip;
use serde_json::{Value, json};

use super::record_tool_trace;
use crate::llm_client_port::WorkflowAgentToolUse;
use crate::v2::result::WorkflowV2Result;

/// `toolTrace.topLevelLists` when `files_read` / `commands_run` are what the
/// agent reported about itself.
pub const AGENT_REPORTED: &str = "agent_reported";

/// The most observed records of each kind kept in `data.toolTrace`. A
/// script view spreads `result.data` and scripts pass results on into
/// later agents' inputs, so the trace kept there stays small; the counts
/// say how many there were.
const MAX_OBSERVED_IN_DATA: usize = 40;
/// The most unobserved claimed paths named in the claim check.
const MAX_UNOBSERVED_NAMED: usize = 20;
/// The most bytes of one named path.
const PATH_BYTES: usize = 240;

/// Stamp a structured (agent-reported) result with what the host trace saw.
///
/// The agent's own `files_read` and `commands_run` stay where they are, and
/// `toolTrace.topLevelLists` says they are agent-reported: verification
/// gates read their `kind`, `exit_code`, `pre_existing` and output text,
/// which a trace record never carries (it stores no tool output), so
/// swapping them would let failing tests through and reject good results.
/// The observed records sit beside them in `toolTrace.filesRead` /
/// `toolTrace.commandsRun` (the first [`MAX_OBSERVED_IN_DATA`] of each), or
/// are marked [`super::NOT_RECORDED`] when no trace was captured.
///
/// With a trace, `toolTrace.claimCheck` compares the agent's claimed
/// `files_read` with the reads the trace observed and names every claim no
/// read matches, so a claimed read the session never made is visible, not
/// silent. `traceComplete` says whether the trace can be missing reads.
///
/// `toolTrace` is host-owned: an agent-written one is replaced. No evidence
/// entry is added, since gates count evidence. A `data` that is neither null
/// nor an object is left as the agent wrote it.
pub fn record_structured_trace(
    result: &mut WorkflowV2Result,
    tool_uses: Option<&[WorkflowAgentToolUse]>,
) {
    if !(result.data.is_null() || result.data.is_object()) {
        return;
    }
    let mut observed = WorkflowV2Result::default();
    record_tool_trace(&mut observed, tool_uses.unwrap_or_default());
    let mut marker = observed.data["toolTrace"].take();
    if marker["recorded"] == json!(true) {
        let observed_paths: Vec<&str> = observed
            .files_read
            .iter()
            .map(|file| file.path.as_str())
            .collect();
        let unobserved: Vec<String> = result
            .files_read
            .iter()
            .map(|file| file.path.trim())
            .filter(|claimed| !claimed.is_empty())
            .filter(|claimed| !observed_paths.iter().any(|seen| same_file(seen, claimed)))
            .map(|claimed| clip(&redact_secret_values(claimed), PATH_BYTES))
            .collect();
        let claimed = result
            .files_read
            .iter()
            .filter(|file| !file.path.trim().is_empty())
            .count();
        marker["claimCheck"] = json!({
            "filesRead": {
                "claimed": result.files_read.len(),
                // Non-empty claims an observed read matches: the only
                // claimed reads a gate may take as proof of inspection.
                "confirmed": claimed - unobserved.len(),
                "observedReads": observed.files_read.len(),
                "unobservedCount": unobserved.len(),
                "unobserved": unobserved.iter().take(MAX_UNOBSERVED_NAMED).collect::<Vec<_>>(),
                "traceComplete": marker["complete"] == json!(true),
            },
            "claimsMatchTrace": unobserved.is_empty(),
        });
        if !unobserved.is_empty() {
            tracing::warn!(
                unobserved = unobserved.len(),
                "agent-reported files_read names files the session trace shows no read of"
            );
        }
        marker["filesReadTotal"] = json!(observed.files_read.len());
        marker["commandsRunTotal"] = json!(observed.commands_run.len());
        observed.files_read.truncate(MAX_OBSERVED_IN_DATA);
        observed.commands_run.truncate(MAX_OBSERVED_IN_DATA);
        marker["filesRead"] = json!(observed.files_read);
        marker["commandsRun"] = json!(observed.commands_run);
    }
    marker["topLevelLists"] = json!(AGENT_REPORTED);
    if result.data.is_null() {
        result.data = json!({});
    }
    result.data["toolTrace"] = marker;
}

/// Whether an observed read path and a claimed path name the same file. A
/// Read call names an absolute path, while an agent usually reports one
/// relative to the repository, so a claim matches an observed path that is
/// it or ends with it at a path boundary.
fn same_file(observed: &str, claimed: &str) -> bool {
    let observed = observed.trim().trim_start_matches("./");
    let claimed = claimed.trim().trim_start_matches("./");
    observed == claimed
        || observed
            .strip_suffix(claimed)
            .is_some_and(|prefix| prefix.ends_with('/'))
}

/// One trace for a call whose answer took several sessions (first answer,
/// repairs, a restarted agent): every call in order, and one summary that
/// sums theirs. When any session's history was not captured (no summary),
/// the merged trace carries no summary, so it is never claimed complete.
/// `None` when no session returned.
pub fn merge_session_traces(
    sessions: Vec<Vec<WorkflowAgentToolUse>>,
) -> Option<Vec<WorkflowAgentToolUse>> {
    if sessions.is_empty() {
        return None;
    }
    let mut calls = Vec::new();
    let mut totals: Option<serde_json::Map<String, Value>> = Some(Default::default());
    for session in sessions {
        let mut summary = None;
        for tool in session {
            if tool.tool_name == TOOL_TRACE_SUMMARY_NAME {
                summary = Some(tool.input);
            } else {
                calls.push(tool);
            }
        }
        totals = match (totals, summary) {
            (Some(mut totals), Some(summary)) => {
                for key in ["calls", "kept", "dropped", "inputs_truncated"] {
                    let add = summary.get(key).and_then(Value::as_u64).unwrap_or(0);
                    let sum = totals.get(key).and_then(Value::as_u64).unwrap_or(0) + add;
                    totals.insert(key.to_string(), json!(sum));
                }
                Some(totals)
            }
            _ => None,
        };
    }
    if let Some(totals) = totals {
        calls.push(WorkflowAgentToolUse {
            tool_name: TOOL_TRACE_SUMMARY_NAME.to_string(),
            input: Value::Object(totals),
            output: Value::Null,
        });
    }
    Some(calls)
}

#[cfg(test)]
#[path = "tool_trace_structured_tests.rs"]
mod tests;
