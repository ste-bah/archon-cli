//! Which claimed reads a gate may take as PROOF of inspection (Issue 276).
//!
//! No false green: when the host captured the session's tool trace, a
//! claimed `files_read` entry proves an inspection only when an observed
//! read matches it (`toolTrace.claimCheck`, written by the host). An
//! unmatched claim is not proof. With no trace captured the agent's claims
//! are all there is; they still count, but the proof says it is
//! agent-reported.
use serde_json::{Value, json};

use super::WorkflowV2Result;

/// `noopProof.filesRead` when the claimed reads were matched by the trace.
pub const TRACE_CONFIRMED: &str = "trace_confirmed";
/// `noopProof.filesRead` when no trace could confirm the claimed reads.
pub const AGENT_REPORTED: &str = "agent_reported";

/// The most unmatched claims a failure names.
const NAMED: usize = 10;

/// What the claimed reads of `result` prove.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReadProof {
    /// No trace was captured: the claims are agent-reported.
    AgentReported,
    /// The trace matched `confirmed` claims; `unmatched` names others.
    Checked {
        confirmed: u64,
        unmatched: Vec<String>,
        unmatched_count: u64,
    },
}

impl ReadProof {
    pub fn of(result: &WorkflowV2Result) -> Self {
        let trace = &result.data["toolTrace"];
        let check = &trace["claimCheck"]["filesRead"];
        if trace["recorded"] != json!(true) || !check.is_object() {
            return Self::AgentReported;
        }
        let count = |key: &str| check[key].as_u64().unwrap_or(0);
        Self::Checked {
            confirmed: count("confirmed"),
            unmatched: check["unobserved"]
                .as_array()
                .map(|paths| {
                    paths
                        .iter()
                        .filter_map(Value::as_str)
                        .take(NAMED)
                        .map(str::to_string)
                        .collect()
                })
                .unwrap_or_default(),
            unmatched_count: count("unobservedCount"),
        }
    }

    /// The unmatched claims as one line: the first few by name, then how
    /// many more.
    pub fn unmatched_line(&self) -> String {
        let Self::Checked {
            unmatched,
            unmatched_count,
            ..
        } = self
        else {
            return String::new();
        };
        let more = unmatched_count.saturating_sub(unmatched.len() as u64);
        let mut line = unmatched.join(", ");
        if more > 0 {
            line.push_str(&format!(" (+{more} more)"));
        }
        line
    }

    /// Stamp `data.noopProof` on a result this proof accepted.
    pub fn stamp(&self, result: &mut WorkflowV2Result) {
        if !(result.data.is_null() || result.data.is_object()) {
            return;
        }
        let proof = match self {
            Self::AgentReported => json!({"filesRead": AGENT_REPORTED}),
            Self::Checked {
                confirmed,
                unmatched_count,
                ..
            } => json!({
                "filesRead": TRACE_CONFIRMED,
                "confirmed": confirmed,
                "unmatchedClaims": unmatched_count,
            }),
        };
        if result.data.is_null() {
            result.data = json!({});
        }
        result.data["noopProof"] = proof;
    }
}
