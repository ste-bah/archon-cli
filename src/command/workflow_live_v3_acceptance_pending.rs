use std::collections::BTreeSet;

use archon_workflow::task_set_contract::AcceptanceCriterion;
use archon_workflow::v2::acceptance_stage::{
    AUTHORED_PENDING_PREFIX, AcceptanceCheckRecordV1, AcceptanceCheckStatus,
    AcceptanceRoundRecordV1,
};

pub(super) fn ids_from_record(record: &AcceptanceRoundRecordV1) -> BTreeSet<String> {
    record
        .contract_repairs
        .iter()
        .filter(|repair| !repair.repaired && repair.failure.starts_with(AUTHORED_PENDING_PREFIX))
        .flat_map(|repair| repair.check_ids.iter().cloned())
        .collect()
}

pub(super) fn extend_ids(ids: &mut BTreeSet<String>, record: &AcceptanceRoundRecordV1) {
    ids.extend(ids_from_record(record));
}

pub(super) fn record(criterion: &AcceptanceCriterion, kind: &str) -> AcceptanceCheckRecordV1 {
    AcceptanceCheckRecordV1 {
        check_id: criterion.id.clone(),
        criterion: criterion.criterion.clone(),
        kind: kind.to_string(),
        status: AcceptanceCheckStatus::Pending,
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
    }
}
