use std::collections::BTreeMap;

use archon_completion::models::{CompletionEvidence, CompletionState, VerificationGateResult};
use archon_completion::{EvidenceKind, EvidenceStatus};
use cozo::{DataValue, MultiTransaction};

use super::plan_store_evidence_writes::{
    insert_authoritative_execution_record, insert_completion_evidence_in, insert_gate_in,
};
use super::{PlanApprovalAuthority, PlanStore, db_err};

pub(super) const EXECUTED_TEST_GATE: &str = "ExecutedTestCommandVerifier";

pub(super) struct AuthoritativeExecutionRecord {
    pub(super) run_id: String,
    pub(super) session_id: String,
    pub(super) tool_use_id: String,
    pub(super) attempt: i64,
    pub(super) command_hash: String,
    pub(super) output_hash: String,
    pub(super) exit_code: i64,
    pub(super) signature: String,
    pub(super) completed_at: String,
}

impl PlanStore {
    /// Atomically persist one opaque Bash execution and its completion evidence.
    #[allow(clippy::too_many_arguments)]
    pub fn record_authoritative_test_execution(
        &self,
        authority: &PlanApprovalAuthority,
        session_id: &str,
        tool_use_id: &str,
        attempt: u32,
        command: &str,
        output: &str,
        exit_code: i32,
    ) -> Result<CompletionEvidence, std::io::Error> {
        archon_completion::schema::ensure_completion_schema(&self.db).map_err(db_err)?;
        let run_id = format!("{session_id}:{tool_use_id}");
        let completed_at = chrono::Utc::now().to_rfc3339();
        let command_hash = hash(command);
        let output_hash = hash(output);
        let output_summary: String = output.chars().take(600).collect();
        let summary_hash = hash(&output_summary);
        let evidence = CompletionEvidence {
            evidence_id: format!("ev-{}", uuid::Uuid::new_v4().simple()),
            run_id: run_id.clone(),
            evidence_kind: EvidenceKind::TestRun,
            producer: "authoritative-bash-execution".into(),
            command_or_operation: Some(command.to_string()),
            status: if exit_code == 0 {
                EvidenceStatus::Passed
            } else {
                EvidenceStatus::Failed
            },
            exit_code: Some(exit_code),
            input_hash: Some(command_hash.clone()),
            output_hash: Some(output_hash.clone()),
            stdout_summary: Some(output_summary.clone()),
            stderr_summary: (exit_code != 0).then(|| output_summary.clone()),
            artifact_ids: vec![],
            provenance_record_id: format!("bash-execution:{session_id}:{tool_use_id}:{attempt}"),
            started_at: completed_at.clone(),
            completed_at: Some(completed_at.clone()),
        };
        let transaction = self.db.multi_transaction(true);
        let result = (|| {
            let signature = self.evidence_signature_in(
                &transaction,
                authority,
                session_id,
                &execution_payload(
                    &evidence.provenance_record_id,
                    &run_id,
                    session_id,
                    tool_use_id,
                    i64::from(attempt),
                    &command_hash,
                    &output_hash,
                    &summary_hash,
                    i64::from(exit_code),
                    &completed_at,
                ),
            )?;
            insert_authoritative_execution_record(
                &transaction,
                &evidence,
                session_id,
                tool_use_id,
                attempt,
                &command_hash,
                &output_hash,
                &signature,
            )?;
            #[cfg(any(test, feature = "test-support"))]
            if self
                .fail_next_authoritative_evidence_after_execution_write
                .swap(false, std::sync::atomic::Ordering::SeqCst)
            {
                return Err(std::io::Error::other(
                    "injected authoritative evidence persistence failure",
                ));
            }
            insert_completion_evidence_in(&transaction, &evidence)
        })();
        self.finish_transaction(transaction, result)?;
        Ok(evidence)
    }

    /// Verify signed Bash evidence and persist its verifier gate in one transaction.
    pub fn verify_test_command_evidence(
        &self,
        authority: &PlanApprovalAuthority,
        session_id: &str,
        run_id: &str,
        evidence_id: &str,
    ) -> Result<VerificationGateResult, std::io::Error> {
        archon_completion::schema::ensure_completion_schema(&self.db).map_err(db_err)?;
        let evidence = archon_completion::store::get_evidence_by_run(&self.db, run_id)
            .map_err(db_err)?
            .into_iter()
            .find(|candidate| candidate.evidence_id == evidence_id)
            .ok_or_else(|| {
                std::io::Error::new(std::io::ErrorKind::NotFound, "test evidence not found")
            })?;
        require_passing_test_evidence(&evidence)?;
        let transaction = self.db.multi_transaction(true);
        let result = (|| {
            let execution =
                load_authoritative_execution_in(&transaction, &evidence.provenance_record_id)?;
            validate_execution_identity(&execution, &evidence, session_id)?;
            self.verify_evidence_signature_in(
                &transaction,
                authority,
                session_id,
                &execution_payload(
                    &evidence.provenance_record_id,
                    &execution.run_id,
                    &execution.session_id,
                    &execution.tool_use_id,
                    execution.attempt,
                    &execution.command_hash,
                    &execution.output_hash,
                    evidence
                        .stdout_summary
                        .as_deref()
                        .map(hash)
                        .as_deref()
                        .unwrap_or_default(),
                    execution.exit_code,
                    &execution.completed_at,
                ),
                &execution.signature,
            )?;
            if !evidence
                .stdout_summary
                .as_deref()
                .is_some_and(has_test_success_signal)
            {
                return Err(untrusted("test output has no successful runner summary"));
            }
            let gate = VerificationGateResult {
                gate_id: format!("gate-{}", uuid::Uuid::new_v4().simple()),
                gate_name: EXECUTED_TEST_GATE.into(),
                passed: true,
                resulting_state: CompletionState::Verified,
                blocked_claims: vec![],
                required_missing_evidence: vec![],
                explanation:
                    "signed Bash execution has exit code 0 and a successful runner summary".into(),
                provenance_record_id: evidence.provenance_record_id.clone(),
            };
            insert_gate_in(&transaction, &gate, run_id)?;
            Ok(gate)
        })();
        finish_value_transaction(transaction, result)
    }
}

fn require_passing_test_evidence(evidence: &CompletionEvidence) -> Result<(), std::io::Error> {
    if evidence.evidence_kind != EvidenceKind::TestRun {
        return Err(untrusted("evidence is not a test run"));
    }
    if evidence.status != EvidenceStatus::Passed || evidence.exit_code != Some(0) {
        return Err(untrusted("test command did not pass"));
    }
    Ok(())
}

fn validate_execution_identity(
    execution: &AuthoritativeExecutionRecord,
    evidence: &CompletionEvidence,
    session_id: &str,
) -> Result<(), std::io::Error> {
    let expected_run_id = format!("{}:{}", execution.session_id, execution.tool_use_id);
    if execution.session_id != session_id
        || execution.run_id != expected_run_id
        || execution.run_id != evidence.run_id
        || execution.attempt < 0
        || execution.command_hash != evidence.input_hash.as_deref().unwrap_or_default()
        || execution.output_hash != evidence.output_hash.as_deref().unwrap_or_default()
        || execution.exit_code != i64::from(evidence.exit_code.unwrap_or(-1))
        || execution.completed_at != evidence.completed_at.as_deref().unwrap_or_default()
    {
        return Err(untrusted(
            "test evidence identity does not match its authoritative Bash execution",
        ));
    }
    Ok(())
}

fn load_authoritative_execution_in(
    transaction: &MultiTransaction,
    provenance: &str,
) -> Result<AuthoritativeExecutionRecord, std::io::Error> {
    let mut params = BTreeMap::new();
    params.insert("provenance".into(), DataValue::from(provenance));
    let rows = transaction
        .run_script(
            "?[run_id, session_id, tool_use_id, attempt, command_hash, output_hash, exit_code, signature, completed_at] := *authoritative_bash_executions{provenance_record_id, run_id, session_id, tool_use_id, attempt, command_hash, output_hash, exit_code, signature, completed_at}, provenance_record_id = $provenance",
            params,
        )
        .map_err(db_err)?;
    let row = rows
        .rows
        .first()
        .filter(|_| rows.rows.len() == 1)
        .ok_or_else(|| untrusted("authoritative Bash execution record is missing or ambiguous"))?;
    Ok(AuthoritativeExecutionRecord {
        run_id: required_string(row, 0)?,
        session_id: required_string(row, 1)?,
        tool_use_id: required_string(row, 2)?,
        attempt: required_int(row, 3, "attempt")?,
        command_hash: required_string(row, 4)?,
        output_hash: required_string(row, 5)?,
        exit_code: required_int(row, 6, "exit code")?,
        signature: required_string(row, 7)?,
        completed_at: required_string(row, 8)?,
    })
}

fn required_string(row: &[DataValue], index: usize) -> Result<String, std::io::Error> {
    row.get(index)
        .and_then(DataValue::get_str)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .ok_or_else(|| untrusted("authoritative Bash execution record is malformed"))
}

fn required_int(row: &[DataValue], index: usize, name: &str) -> Result<i64, std::io::Error> {
    row.get(index)
        .and_then(DataValue::get_int)
        .ok_or_else(|| untrusted(format!("invalid {name}")))
}

#[allow(clippy::too_many_arguments)]
pub(super) fn execution_payload(
    provenance: &str,
    run_id: &str,
    session_id: &str,
    tool_use_id: &str,
    attempt: i64,
    command_hash: &str,
    output_hash: &str,
    success_summary_hash: &str,
    exit_code: i64,
    completed_at: &str,
) -> Vec<u8> {
    let mut payload = Vec::new();
    for value in [
        provenance,
        run_id,
        session_id,
        tool_use_id,
        &attempt.to_string(),
        command_hash,
        output_hash,
        success_summary_hash,
        &exit_code.to_string(),
        completed_at,
    ] {
        payload.extend_from_slice(&(value.len() as u64).to_be_bytes());
        payload.extend_from_slice(value.as_bytes());
    }
    payload
}

fn hash(value: &str) -> String {
    use sha2::{Digest, Sha256};
    format!("{:x}", Sha256::digest(value.as_bytes()))
}

fn has_test_success_signal(output: &str) -> bool {
    let lower = output.to_ascii_lowercase();
    let cargo_summary = lower
        .lines()
        .any(|line| line.trim_start().starts_with("test result: ok.") && line.contains("0 failed"));
    let pytest_summary = lower.lines().any(|line| {
        let line = line.trim();
        line.contains(" passed in ") && !line.contains(" failed") && !line.contains(" error")
    });
    let javascript_summary = lower.lines().any(|line| {
        let line = line.trim_start();
        (line.starts_with("tests:") || line.starts_with("test suites:"))
            && line.contains("passed")
            && !line.contains("failed")
    });
    cargo_summary || pytest_summary || javascript_summary
}

fn finish_value_transaction<T>(
    transaction: MultiTransaction,
    result: Result<T, std::io::Error>,
) -> Result<T, std::io::Error> {
    match result {
        Ok(value) => {
            transaction.commit().map_err(db_err)?;
            Ok(value)
        }
        Err(error) => {
            let _ = transaction.abort();
            Err(error)
        }
    }
}

pub(super) fn untrusted(message: impl AsRef<str>) -> std::io::Error {
    std::io::Error::new(std::io::ErrorKind::PermissionDenied, message.as_ref())
}
