use std::collections::BTreeSet;

use archon_workflow::acceptance_scratch::CheckResult;
use archon_workflow::task_set_contract::AcceptanceCriterion;
use archon_workflow::v2::acceptance_stage::{
    AUTHORED_PENDING_PREFIX, AcceptanceCheckRecordV1, AcceptanceCheckStatus,
    AcceptanceRoundRecordV1,
};

use super::repair::HOST_PENDING_PREFIX;

pub(super) fn ids_from_record(record: &AcceptanceRoundRecordV1) -> BTreeSet<String> {
    record
        .contract_repairs
        .iter()
        .filter(|repair| {
            !repair.repaired
                && (repair.failure.starts_with(AUTHORED_PENDING_PREFIX)
                    || repair.failure.starts_with(HOST_PENDING_PREFIX))
        })
        .flat_map(|repair| repair.check_ids.iter().cloned())
        .collect()
}

pub(super) fn extend_ids(ids: &mut BTreeSet<String>, record: &AcceptanceRoundRecordV1) {
    ids.extend(ids_from_record(record));
}

pub(super) fn record_from_round(
    criterion: &AcceptanceCriterion,
    kind: &str,
    result: Option<&CheckResult>,
    round: &AcceptanceRoundRecordV1,
) -> AcceptanceCheckRecordV1 {
    let reason = round
        .contract_repairs
        .iter()
        .find(|repair| !repair.repaired && repair.check_ids.contains(&criterion.id))
        .map(|repair| repair.failure.as_str());
    record(criterion, kind, result, reason)
}

pub(super) fn record(
    criterion: &AcceptanceCriterion,
    kind: &str,
    result: Option<&CheckResult>,
    reason: Option<&str>,
) -> AcceptanceCheckRecordV1 {
    AcceptanceCheckRecordV1 {
        check_id: criterion.id.clone(),
        criterion: criterion.criterion.clone(),
        kind: kind.to_string(),
        status: AcceptanceCheckStatus::Pending,
        exit_code: result.and_then(|result| result.exit_code),
        operational_error: reason.map(str::to_string),
        owning_tasks: Vec::new(),
        stdout_tail: result.map_or_else(String::new, |result| super::output::tail(&result.stdout)),
        stderr_tail: result.map_or_else(String::new, |result| super::output::tail(&result.stderr)),
        regressed_by: None,
        contract_defect: false,
        routing: None,
        regression_search: None,
        blocked: None,
    }
}
