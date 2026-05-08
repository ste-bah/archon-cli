use anyhow::Result;
use serde::{Deserialize, Serialize};

use crate::audit::store::PipelineBundleStore;
use crate::audit::verify::verify_bundle;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TraceExportRow {
    pub session_id: String,
    pub agent_key: String,
    pub ordinal: usize,
    pub phase: u32,
    pub action: String,
    pub attempt: usize,
    pub accepted: bool,
    pub failure_reason: Option<String>,
    pub prompt_hash: String,
    pub output_hash: String,
    pub tokens_in: u64,
    pub tokens_out: u64,
    pub quality_overall: Option<f64>,
    pub verifier_status: String,
}

pub fn export_jsonl(
    store: &PipelineBundleStore,
    session_id: &str,
    include_unverified: bool,
) -> Result<String> {
    let verification = verify_bundle(store, session_id, false)?;
    if !verification.valid && !include_unverified {
        anyhow::bail!("bundle is not verified; use --include-unverified to export anyway");
    }
    let status = if verification.valid {
        "verified"
    } else {
        "unverified"
    };
    let mut out = String::new();
    for record in store.list_agent_records(session_id)? {
        for attempt in record.attempts.iter() {
            let row = TraceExportRow {
                session_id: session_id.to_string(),
                agent_key: record.agent_key.clone(),
                ordinal: record.ordinal,
                phase: record.phase,
                action: "agent_attempt".into(),
                attempt: attempt.attempt,
                accepted: attempt.accepted,
                failure_reason: attempt.failure_reason.clone(),
                prompt_hash: record.prompt_hash.clone(),
                output_hash: attempt.output_hash.clone(),
                tokens_in: attempt.tokens_in,
                tokens_out: attempt.tokens_out,
                quality_overall: attempt.quality.as_ref().map(|q| q.overall),
                verifier_status: status.into(),
            };
            out.push_str(&serde_json::to_string(&row)?);
            out.push('\n');
        }
    }
    Ok(out)
}
