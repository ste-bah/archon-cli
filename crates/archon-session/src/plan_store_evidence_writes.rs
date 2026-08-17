use std::collections::BTreeMap;

use archon_completion::models::{CompletionEvidence, EvidenceStatus, VerificationGateResult};
use cozo::{DataValue, MultiTransaction};

use super::db_err;

#[allow(clippy::too_many_arguments)]
pub(super) fn insert_authoritative_execution_record(
    transaction: &MultiTransaction,
    evidence: &CompletionEvidence,
    session_id: &str,
    tool_use_id: &str,
    attempt: u32,
    command_hash: &str,
    output_hash: &str,
    signature: &str,
) -> Result<(), std::io::Error> {
    let mut params = BTreeMap::new();
    params.insert(
        "provenance".into(),
        DataValue::from(evidence.provenance_record_id.as_str()),
    );
    params.insert("run".into(), DataValue::from(evidence.run_id.as_str()));
    params.insert("session".into(), DataValue::from(session_id));
    params.insert("tool".into(), DataValue::from(tool_use_id));
    params.insert("attempt".into(), DataValue::from(i64::from(attempt)));
    params.insert("command".into(), DataValue::from(command_hash));
    params.insert("output".into(), DataValue::from(output_hash));
    params.insert(
        "exit".into(),
        DataValue::from(i64::from(evidence.exit_code.unwrap_or(-1))),
    );
    params.insert("signature".into(), DataValue::from(signature));
    params.insert(
        "completed".into(),
        DataValue::from(evidence.completed_at.as_deref().unwrap_or_default()),
    );
    transaction
        .run_script(
            "?[provenance_record_id, run_id, session_id, tool_use_id, attempt, command_hash, output_hash, exit_code, signature, completed_at] <- [[$provenance, $run, $session, $tool, $attempt, $command, $output, $exit, $signature, $completed]] :insert authoritative_bash_executions {provenance_record_id => run_id, session_id, tool_use_id, attempt, command_hash, output_hash, exit_code, signature, completed_at}",
            params,
        )
        .map_err(db_err)?;
    Ok(())
}

pub(super) fn insert_completion_evidence_in(
    transaction: &MultiTransaction,
    evidence: &CompletionEvidence,
) -> Result<(), std::io::Error> {
    let mut params = BTreeMap::new();
    params.insert("eid".into(), DataValue::from(evidence.evidence_id.as_str()));
    params.insert("rid".into(), DataValue::from(evidence.run_id.as_str()));
    params.insert("ek".into(), DataValue::from("TestRun"));
    params.insert("pr".into(), DataValue::from(evidence.producer.as_str()));
    params.insert(
        "st".into(),
        DataValue::from(if evidence.status == EvidenceStatus::Passed {
            "Passed"
        } else {
            "Failed"
        }),
    );
    params.insert(
        "ec".into(),
        DataValue::from(i64::from(evidence.exit_code.unwrap_or(-1))),
    );
    params.insert(
        "ih".into(),
        DataValue::from(evidence.input_hash.as_deref().unwrap_or_default()),
    );
    params.insert(
        "oh".into(),
        DataValue::from(evidence.output_hash.as_deref().unwrap_or_default()),
    );
    params.insert(
        "cmd".into(),
        DataValue::from(evidence.command_or_operation.as_deref().unwrap_or_default()),
    );
    params.insert(
        "out".into(),
        DataValue::from(evidence.stdout_summary.as_deref().unwrap_or_default()),
    );
    params.insert(
        "err".into(),
        DataValue::from(evidence.stderr_summary.as_deref().unwrap_or_default()),
    );
    let artifacts = serde_json::to_string(&evidence.artifact_ids).map_err(db_err)?;
    params.insert("aj".into(), DataValue::from(artifacts.as_str()));
    params.insert(
        "prid".into(),
        DataValue::from(evidence.provenance_record_id.as_str()),
    );
    params.insert("sa".into(), DataValue::from(evidence.started_at.as_str()));
    params.insert(
        "coa".into(),
        DataValue::from(evidence.completed_at.as_deref().unwrap_or_default()),
    );
    transaction
        .run_script(
            "?[evidence_id, run_id, evidence_kind, producer, status, exit_code, input_hash, output_hash, command_or_operation, stdout_summary, stderr_summary, artifact_ids_json, provenance_record_id, started_at, completed_at] <- [[$eid, $rid, $ek, $pr, $st, $ec, $ih, $oh, $cmd, $out, $err, $aj, $prid, $sa, $coa]] :insert completion_evidence {evidence_id => run_id, evidence_kind, producer, status, exit_code, input_hash, output_hash, command_or_operation, stdout_summary, stderr_summary, artifact_ids_json, provenance_record_id, started_at, completed_at}",
            params,
        )
        .map_err(db_err)?;
    Ok(())
}

pub(super) fn insert_gate_in(
    transaction: &MultiTransaction,
    gate: &VerificationGateResult,
    run_id: &str,
) -> Result<(), std::io::Error> {
    let mut params = BTreeMap::new();
    params.insert("gid".into(), DataValue::from(gate.gate_id.as_str()));
    params.insert("rid".into(), DataValue::from(run_id));
    params.insert("name".into(), DataValue::from(gate.gate_name.as_str()));
    params.insert("state".into(), DataValue::from("Verified"));
    params.insert(
        "prov".into(),
        DataValue::from(gate.provenance_record_id.as_str()),
    );
    let now = chrono::Utc::now().to_rfc3339();
    params.insert("now".into(), DataValue::from(now.as_str()));
    transaction
        .run_script(
            "?[gate_id, run_id, gate_name, passed, resulting_state, blocked_claims_json, required_missing_evidence_json, explanation, provenance_record_id, created_at] <- [[$gid, $rid, $name, true, $state, '[]', '[]', 'signed authoritative Bash execution verified', $prov, $now]] :insert verification_gate_results {gate_id => run_id, gate_name, passed, resulting_state, blocked_claims_json, required_missing_evidence_json, explanation, provenance_record_id, created_at}",
            params,
        )
        .map_err(db_err)?;
    Ok(())
}
