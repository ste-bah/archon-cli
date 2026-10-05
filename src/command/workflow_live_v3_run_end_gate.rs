//! The acceptance round the final gate is judged on (Obs-32), read so that
//! a damaged bound record never errors over a run left Running (Issue 262,
//! round 9).
//!
//! The gate is bound to the record the run's last acceptance call names.
//! That record can be damaged after the call finished, or already moved to
//! quarantine by the healing load (whose ledger copy keeps only the failing
//! ids, too little for a gate). The call's own result is a whole second
//! copy of the round -- its number and attempt, whether the contract was
//! present, every failing check with its owners, the passed ids and the
//! operational errors -- so the gate is rebuilt from it, with a warning
//! and an `acceptance_gate_rebuilt` event. When it is no whole copy either,
//! or the record cannot be read (an I/O fault), the run PAUSES with the
//! reason: never failed, never left Running.

use archon_workflow::v2::acceptance_stage::{
    ACCEPTANCE_ROUND_RECORD_SCHEMA_VERSION, AcceptanceCheckRecordV1, AcceptanceCheckStatus,
    AcceptanceRoundRecordV1, attempt_file_name, latest_round_record, relative_record_path,
    round_dir,
};
use archon_workflow::v2::script::is_acceptance_stage_call;
use archon_workflow::{
    AuthoredAcceptanceGateV1, WorkflowEventKind, WorkflowEventLog, WorkflowResult, WorkflowStore,
    WorkflowV2ResultStore,
};

/// The acceptance round the gate is judged on, and whether it is BOUND to a
/// call this run executed or replayed.
pub(super) struct GateRecord {
    pub(super) gate: AuthoredAcceptanceGateV1,
    pub(super) record: AcceptanceRoundRecordV1,
    pub(super) path: std::path::PathBuf,
    pub(super) bound: bool,
}

/// The bound record as found on disk.
enum Bound {
    Whole(Box<AcceptanceRoundRecordV1>),
    /// Damaged or gone: why.
    Lost(String),
    /// The file system would not hand it over.
    Unreadable(std::io::Error),
}

fn read_bound(path: &std::path::Path) -> Bound {
    match std::fs::read(path) {
        Ok(bytes) => match serde_json::from_slice(&bytes) {
            Ok(record) => Bound::Whole(Box::new(record)),
            Err(error) => Bound::Lost(format!("it will not parse ({error})")),
        },
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            Bound::Lost("it is gone (quarantined as damaged, or removed)".to_string())
        }
        Err(error) => Bound::Unreadable(error),
    }
}

/// The round record named by the last acceptance call in `calls` (its own
/// record's `data.record_path`), so a record an earlier process left for a
/// round this run never reached can neither pass nor pin the gate. A run
/// with no acceptance call (an older script) falls back to the newest record
/// on disk, unbound. A damaged or missing bound record is rebuilt from the
/// call's result; `Err(ControlPaused)` when it cannot be (the run is
/// paused).
pub(super) fn read_acceptance_gate(
    store: &WorkflowStore,
    run_id: &str,
    v2_store: &WorkflowV2ResultStore,
    calls: &[archon_workflow::WorkflowV2HostCall],
) -> WorkflowResult<Option<GateRecord>> {
    let run_dir = store.run_dir(run_id);
    let Some(call) = calls
        .iter()
        .rev()
        .find(|call| is_acceptance_stage_call(call))
    else {
        let latest = latest_round_record(&run_dir)?;
        return Ok(latest.map(|(record, path)| gate_record(&run_dir, record, path, false)));
    };
    // Issue 313: a damaged or unreadable call record pauses the run.
    let Some(result) = super::call::acceptance_call_record(store, run_id, v2_store, call)?
        .map(|record| record.result)
    else {
        return Ok(None);
    };
    let Some(named) = (result.data.get("record_path")).and_then(serde_json::Value::as_str) else {
        return Ok(None);
    };
    let path = run_dir.join(named);
    let record = match read_bound(&path) {
        Bound::Whole(record) => *record,
        Bound::Unreadable(error) => {
            let reason = format!("the bound acceptance record {named} cannot be read ({error})");
            return Err(super::call::pause(store, run_id, named, &reason));
        }
        Bound::Lost(why) => match rebuild(&run_dir, run_id, &call.id, named, &result.data) {
            Ok(record) => {
                rebuilt(store, run_id, &call.id, named, &why);
                record
            }
            Err(missing) => {
                let reason = format!(
                    "the bound acceptance record {named} is lost: {why}; the acceptance call's result is no whole copy of the round ({missing})"
                );
                return Err(super::call::pause(store, run_id, named, &reason));
            }
        },
    };
    Ok(Some(gate_record(&run_dir, record, path, true)))
}

fn gate_record(
    run_dir: &std::path::Path,
    record: AcceptanceRoundRecordV1,
    path: std::path::PathBuf,
    bound: bool,
) -> GateRecord {
    GateRecord {
        gate: gate_of(run_dir, &record, &path),
        record,
        path,
        bound,
    }
}

/// The round `named` rebuilt from its acceptance call's result `data`;
/// `Err` names what the result lacks.
fn rebuild(
    run_dir: &std::path::Path,
    run_id: &str,
    call_id: &str,
    named: &str,
    data: &serde_json::Value,
) -> Result<AcceptanceRoundRecordV1, String> {
    fn field<T: serde::de::DeserializeOwned>(
        data: &serde_json::Value,
        name: &str,
    ) -> Result<T, String> {
        let value = data.get(name).ok_or_else(|| format!("no `{name}`"))?;
        serde_json::from_value(value.clone()).map_err(|error| format!("`{name}`: {error}"))
    }
    let (round, attempt): (u32, u32) = (field(data, "round")?, field(data, "attempt")?);
    let at = relative_record_path(
        run_dir,
        &round_dir(run_dir, round).join(attempt_file_name(attempt)),
    );
    if at != named {
        return Err(format!(
            "it is round {round} attempt {attempt} ({at}), not {named}"
        ));
    }
    let mut checks: Vec<AcceptanceCheckRecordV1> = field(data, "failing")?;
    if let Some(check) = checks.iter().find(|check| !check.failing()) {
        return Err(format!("`failing` lists {} as passed", check.check_id));
    }
    let passed: Vec<String> = field(data, "passed")?;
    checks.extend(passed.into_iter().map(|check_id| AcceptanceCheckRecordV1 {
        check_id,
        criterion: String::new(),
        kind: String::new(),
        status: AcceptanceCheckStatus::Passed,
        exit_code: None,
        operational_error: None,
        owning_tasks: Vec::new(),
        stdout_tail: String::new(),
        stderr_tail: String::new(),
        regressed_by: None,
        contract_defect: false,
        routing: None,
        regression_search: None,
        blocked: None,
    }));
    Ok(AcceptanceRoundRecordV1 {
        schema_version: ACCEPTANCE_ROUND_RECORD_SCHEMA_VERSION,
        run_id: run_id.to_string(),
        call_id: call_id.to_string(),
        round,
        attempt,
        max_rounds: field(data, "max_rounds").unwrap_or_default(),
        contract_present: field(data, "contract_present")?,
        requested_check_ids: Vec::new(),
        execution: None,
        checks,
        operational_errors: field(data, "operational_errors")?,
        contract_repairs: Vec::new(),
        final_round: field(data, "final")?,
    })
}

/// Logs and records that the gate was rebuilt (best effort: the warning
/// is the record when the event log will not take it).
fn rebuilt(store: &WorkflowStore, run_id: &str, call_id: &str, named: &str, why: &str) {
    tracing::warn!(
        run_id,
        record = named,
        call_id,
        "the bound acceptance record is lost ({why}); the final gate is rebuilt from the acceptance call's own result"
    );
    let detail = serde_json::json!({
        "event": "acceptance_gate_rebuilt",
        "record_path": named,
        "why": why,
        "call_id": call_id,
    });
    let kind = WorkflowEventKind::AcceptanceGateRebuilt;
    if let Err(error) = store
        .next_event_seq(run_id)
        .and_then(|seq| WorkflowEventLog::new(store.clone()).emit(run_id, seq, kind, detail))
    {
        tracing::warn!(%error, run_id, "acceptance gate rebuild event not recorded");
    }
}

/// The gate a round record at `path` gives.
pub(super) fn gate_of(
    run_dir: &std::path::Path,
    record: &AcceptanceRoundRecordV1,
    path: &std::path::Path,
) -> AuthoredAcceptanceGateV1 {
    AuthoredAcceptanceGateV1 {
        final_round: record.round,
        attempt: record.attempt,
        record_path: relative_record_path(run_dir, path),
        contract_present: record.contract_present,
        failing_check_ids: record.failing_check_ids(),
        unowned_failing_check_ids: record.unowned_failing_check_ids(),
        operational_errors: record.operational_errors.clone(),
    }
}
