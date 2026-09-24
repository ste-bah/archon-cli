//! Reading a call's review contract: which stage it is, of which kind, and
//! which source maps a reduce names. Shared by the finding attachment, the
//! reduce roster and the host's branch retry, which all key on the contract a
//! call declares rather than on its name.

use serde_json::{Map, Value};

use super::super::call_execution::WorkflowV2CallExecution;

const REVIEW_CONTRACT_KEYS: [&str; 2] = ["reviewContract", "review_contract"];
const SOURCE_MAP_KEYS: [&str; 2] = ["sourceMapCallIds", "source_map_call_ids"];
pub(super) const MAP_STAGE: &str = "map";
pub(super) const REDUCE_FINAL_STAGE: &str = "reduce_final";

/// The review contract a call carries, under either spelling. Shared with
/// `review_roster`, which reads the same contract before the call runs.
pub(crate) fn review_contract(execution: &WorkflowV2CallExecution) -> Option<&Map<String, Value>> {
    REVIEW_CONTRACT_KEYS
        .iter()
        .find_map(|key| execution.call.options.extra.get(*key))
        .and_then(Value::as_object)
}

pub(super) fn contract_string(contract: &Map<String, Value>, key: &str) -> String {
    contract
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .unwrap_or_default()
        .to_string()
}

pub(crate) fn source_map_call_ids(contract: &Map<String, Value>) -> Vec<String> {
    SOURCE_MAP_KEYS
        .iter()
        .find_map(|key| contract.get(*key))
        .and_then(Value::as_array)
        .map(|ids| {
            ids.iter()
                .filter_map(Value::as_str)
                .map(str::trim)
                .filter(|id| !id.is_empty())
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

/// Is this call the map of a review — one branch per reviewed task? Read from
/// the contract the call declares, so it holds for any review kind a script
/// names.
pub fn is_review_map_call(execution: &WorkflowV2CallExecution) -> bool {
    review_contract(execution)
        .is_some_and(|contract| contract_string(contract, "stage") == MAP_STAGE)
}
