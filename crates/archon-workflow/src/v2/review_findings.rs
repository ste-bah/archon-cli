//! The one owner of three rules about review findings: which fields hold
//! them, when two findings are the same, and which task a finding belongs to.
//!
//! These rules used to exist four times -- the host's accounting check, the
//! host's remediation policy, the prelude's JavaScript, and an offline replay
//! -- with nothing in the build that failed when they drifted. Six live runs
//! failed on that drift, each after the run had otherwise succeeded, and each
//! fix on one side opened the next gap on the other.
//!
//! Now the host computes the review finding set from records it already holds
//! and ATTACHES it to the call result under [`HOST_REVIEW_FINDINGS_KEY`]. The
//! script reads that attachment; it never extracts. The accounting the script
//! reports is then compared with what the host attached, so the check is
//! host-against-host and cannot drift.
//!
//! Attribution reads the reviewed task from the branch's own input, which the
//! host built. The prelude keyed a table by the `item_id` it put on each map
//! item, but the host forms branch ids from `id`/`task_id`/`work_unit_id` and
//! never `item_id`, so that table matched no live branch.

use std::collections::{BTreeMap, BTreeSet};

use serde_json::{Map, Value};

use super::WorkflowV2Result;
use super::call_execution::WorkflowV2CallExecution;
use super::outcome_envelope::outcomes_of;
use super::result_store::WorkflowV2ResultStore;
use crate::task_universe::WorkflowV2TaskUniverse;
use crate::{WorkflowError, WorkflowResult};

#[path = "review_contract.rs"]
mod contract;
pub use contract::is_review_map_call;
use contract::{MAP_STAGE, REDUCE_FINAL_STAGE, contract_string};
pub(crate) use contract::{review_contract, source_map_call_ids};
#[path = "review_unreviewed.rs"]
mod unreviewed;
pub use unreviewed::{REVIEW_OUTCOME_KEY, UNREVIEWED_OUTCOME, unreviewed_task_ids};

/// Arrays that carry findings, wherever they sit in an envelope.
pub const FINDINGS_ARRAY_KEYS: [&str; 3] =
    ["findings", "adversarial_findings", "uncovered_requirements"];

/// Fields that identify a finding; sharing any one of them is the same finding.
pub const IDENTITY_KEYS: [&str; 6] = [
    "id",
    "title",
    "claim",
    "summary",
    "finding",
    "requirement_id",
];

/// The `data` field under which the host attaches the review finding set.
pub const HOST_REVIEW_FINDINGS_KEY: &str = "review_findings";

/// Marker on a reduce finding the maps never saw.
pub const CROSS_CUTTING_SCOPE: &str = "cross_cutting";

/// The mandated review kind whose final set carries the baseline-routed
/// findings: the first one the authored script hands to `remediateFindings`.
const BASELINE_FINDINGS_REVIEW_KIND: &str = "adversarial_findings";

/// The routed baseline findings the merged set does not already hold (by
/// finding identity), so a reducer that restated one adds nothing twice.
fn baseline_findings_not_already_present(
    store: &WorkflowV2ResultStore,
    present: &[Value],
) -> Vec<Value> {
    let seen: BTreeSet<String> = present.iter().flat_map(finding_identities).collect();
    crate::v2::write::test_baseline::all_routed_findings(store)
        .into_iter()
        .filter(|finding| {
            !finding_identities(finding)
                .iter()
                .any(|key| seen.contains(key))
        })
        .collect()
}

/// Every finding an envelope carries: the arrays named in
/// [`FINDINGS_ARRAY_KEYS`], recursing through `data` and `result`, then through
/// `outcomes` or -- only when there are none -- `items`, which are two views of
/// the same fan-out branches.
pub fn collect_findings(value: &Value) -> Vec<Value> {
    let mut findings = Vec::new();
    collect_into(value, &mut findings);
    findings
}

fn collect_into(value: &Value, findings: &mut Vec<Value>) {
    match value {
        Value::Array(items) => {
            for item in items {
                collect_into(item, findings);
            }
        }
        Value::Object(object) => {
            for key in FINDINGS_ARRAY_KEYS {
                if let Some(array) = object.get(key).and_then(Value::as_array) {
                    findings.extend(array.iter().cloned());
                }
            }
            for key in ["data", "result"] {
                if let Some(inner) = object.get(key) {
                    collect_into(inner, findings);
                }
            }
            if let Some(outcomes) = object.get("outcomes") {
                collect_into(outcomes, findings);
            } else if let Some(items) = object.get("items") {
                collect_into(items, findings);
            }
        }
        _ => {}
    }
}

/// Every identity a finding carries, as `key:value` pairs. A finding with no
/// identity field has none, and is compared exactly instead.
pub fn finding_identities(finding: &Value) -> Vec<String> {
    let Some(object) = finding.as_object() else {
        return Vec::new();
    };
    IDENTITY_KEYS
        .iter()
        .filter_map(|key| {
            let text = object.get(*key)?.as_str()?.trim();
            (!text.is_empty())
                .then(|| format!("{key}:{}", text.chars().take(200).collect::<String>()))
        })
        .collect()
}

/// A stable key for multiset comparison: the identities, or the whole document
/// for a finding that has none.
pub fn finding_key(finding: &Value) -> String {
    // A bare string is the text the host now wraps as `{claim: <text>}`
    // ([`wrap_bare`]); both spellings are the same finding.
    if let Some(text) = finding.as_str() {
        return format!(
            "claim:{}",
            text.trim().chars().take(200).collect::<String>()
        );
    }
    let identities = finding_identities(finding);
    if identities.is_empty() {
        return finding.to_string();
    }
    identities.join("|")
}

/// What makes a reduce finding a restatement of a map finding: any shared
/// identity, or -- for a finding with no identity field, a bare requirement id
/// say -- the whole document. Without the second half a reducer that echoed
/// a bare-string map finding would double it in the merged set.
fn restatement_keys(finding: &Value) -> Vec<String> {
    let identities = finding_identities(finding);
    if identities.is_empty() {
        return vec![finding_key(finding)];
    }
    identities
}

/// The findings of a review map, each stamped with the task its branch
/// reviewed. A branch's task comes from what the branch itself reported, then
/// from the input the host built for it; a finding the host cannot place is
/// still returned, unattributed, rather than dropped.
pub fn attributed_map_findings(
    map_result_data: &Value,
    item_task_ids: &BTreeMap<String, Vec<String>>,
) -> Vec<Value> {
    attributed_map_findings_in(map_result_data, item_task_ids, None)
}

/// [`attributed_map_findings`], resolving ids against the task universe: a
/// finding whose ids name no universe task is stamped with its branch's task
/// exactly as one that names none.
pub fn attributed_map_findings_in(
    map_result_data: &Value,
    item_task_ids: &BTreeMap<String, Vec<String>>,
    universe: Option<&WorkflowV2TaskUniverse>,
) -> Vec<Value> {
    let mut collected = Vec::new();
    for outcome in outcomes_of(map_result_data) {
        let branch = collect_findings(&outcome);
        if branch.is_empty() {
            continue;
        }
        let mut task_ids = task_ids_of(&outcome);
        if task_ids.is_empty() {
            let item_id = outcome
                .get("item_id")
                .or_else(|| outcome.get("id"))
                .and_then(Value::as_str)
                .unwrap_or_default();
            task_ids = item_task_ids.get(item_id).cloned().unwrap_or_default();
        }
        collected.extend(
            branch
                .into_iter()
                .map(|finding| normalize_task_ids_in(wrap_bare(finding), universe))
                .map(|finding| normalize_task_ids_in(stamp_task_ids(finding, &task_ids), universe)),
        );
    }
    collected
}

/// Re-attach attribution to findings that lost it -- a reducer told to
/// preserve map findings verbatim may drop or rename the field -- by matching
/// identities against the stamped map set.
pub fn reattribute(findings: Vec<Value>, stamped: &[Value]) -> Vec<Value> {
    let mut by_identity: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for finding in stamped {
        let ids = task_ids_of(finding);
        if ids.is_empty() {
            continue;
        }
        for key in finding_identities(finding) {
            by_identity.entry(key).or_insert_with(|| ids.clone());
        }
    }
    findings
        .into_iter()
        .map(|finding| {
            let known = finding_identities(&finding)
                .into_iter()
                .find_map(|key| by_identity.get(&key).cloned());
            match known {
                Some(ids) => stamp_task_ids(finding, &ids),
                None => finding,
            }
        })
        .collect()
}

/// The map findings carried through structurally, plus every reduce finding
/// that is not a restatement of one of them. A restatement shares any identity
/// with a map finding and contributes nothing however it is reworded; a
/// genuinely new reduce finding is marked cross-cutting.
pub fn merge_map_and_reduce(map: Vec<Value>, reduce: Vec<Value>) -> Vec<Value> {
    let mut seen: BTreeSet<String> = map.iter().flat_map(restatement_keys).collect();
    let mut merged = map;
    for finding in reduce {
        let keys = restatement_keys(&finding);
        if keys.iter().any(|key| seen.contains(key)) {
            continue;
        }
        seen.extend(keys);
        merged.push(match finding {
            Value::Object(mut object) => {
                object.insert(
                    "finding_scope".to_string(),
                    Value::from(CROSS_CUTTING_SCOPE),
                );
                Value::Object(object)
            }
            other => other,
        });
    }
    merged
}

/// What the host attaches to a review call's result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostReviewFindings {
    pub kind: String,
    pub stage: String,
    pub findings: Vec<Value>,
    pub map_finding_count: usize,
    pub reduce_finding_count: usize,
    pub source_map_call_ids: Vec<String>,
    /// Source maps the reduce named that have no recorded result. Empty on a
    /// sound run; the accounting check refuses a final reducer that lists any.
    pub missing_source_map_call_ids: Vec<String>,
    /// Findings the write path routed to a task from another branch's
    /// base-commit test baseline (Obs-31), merged into the final reducer's
    /// set so `remediateFindings` acts on them with the review findings.
    pub baseline_finding_count: usize,
}

impl HostReviewFindings {
    fn to_value(&self) -> Value {
        serde_json::json!({
            "source": "host",
            "kind": self.kind,
            "stage": self.stage,
            "findings": self.findings,
            "map_finding_count": self.map_finding_count,
            "reduce_finding_count": self.reduce_finding_count,
            "source_map_call_ids": self.source_map_call_ids,
            "missing_source_map_call_ids": self.missing_source_map_call_ids,
            "baseline_finding_count": self.baseline_finding_count,
            unreviewed::UNREVIEWED_TASK_IDS_KEY: unreviewed_task_ids(&self.findings),
        })
    }
}

fn attach(data: &mut Value, findings: &HostReviewFindings) {
    if !data.is_object() {
        *data = Value::Object(Map::new());
    }
    if let Some(object) = data.as_object_mut() {
        object.insert(HOST_REVIEW_FINDINGS_KEY.to_string(), findings.to_value());
    }
}

/// The task each branch of a review map reviews, keyed by branch id, read from
/// the input the host built for the branch.
fn item_task_ids(
    execution: &WorkflowV2CallExecution,
    store: &WorkflowV2ResultStore,
) -> WorkflowResult<BTreeMap<String, Vec<String>>> {
    let items = super::call_data::fanout_items_for_call(execution, store)?;
    Ok(items
        .into_iter()
        .map(|item| {
            let declared = item.input.get("item").map(task_ids_of).unwrap_or_default();
            (item.id, declared)
        })
        .collect())
}

/// Attach the host-computed review finding set to a call that carries a review
/// contract. A map gets its branch findings attributed; a reduce gets the map
/// findings it named merged with its own. Calls without a contract are left
/// untouched.
pub fn attach_host_review_findings(
    execution: &WorkflowV2CallExecution,
    result: &mut WorkflowV2Result,
    store: &WorkflowV2ResultStore,
) -> WorkflowResult<()> {
    attach_host_review_findings_in(execution, result, store, None)
}

/// [`attach_host_review_findings`], with every attached finding's task ids
/// resolved against the task universe (see `normalize_task_ids_in`).
pub fn attach_host_review_findings_in(
    execution: &WorkflowV2CallExecution,
    result: &mut WorkflowV2Result,
    store: &WorkflowV2ResultStore,
    universe: Option<&WorkflowV2TaskUniverse>,
) -> WorkflowResult<()> {
    let Some(contract) = review_contract(execution) else {
        return Ok(());
    };
    let stage = contract_string(contract, "stage");
    let kind = contract_string(contract, "kind");
    if stage.is_empty() || kind.is_empty() {
        return Err(WorkflowError::SpecInvalid(format!(
            "call '{}' carries a review contract without both `stage` and `kind`",
            execution.call.id
        )));
    }
    let attached_findings = if stage == MAP_STAGE {
        let item_ids = item_task_ids(execution, store)?;
        let mut findings = attributed_map_findings_in(&result.data, &item_ids, universe);
        // A branch that never completed its review is recorded as such, never
        // as the zero findings a clean review reports.
        findings.extend(unreviewed::unreviewed_findings(
            &result.data,
            &item_ids,
            &kind,
        ));
        HostReviewFindings {
            kind,
            stage,
            map_finding_count: findings.len(),
            reduce_finding_count: 0,
            findings,
            source_map_call_ids: Vec::new(),
            missing_source_map_call_ids: Vec::new(),
            baseline_finding_count: 0,
        }
    } else {
        let sources = source_map_call_ids(contract);
        let mut map_findings = Vec::new();
        let mut missing = Vec::new();
        for call_id in &sources {
            match store.load_call_record(call_id)? {
                Some(record) => {
                    map_findings.extend(attached(&record.result.data).unwrap_or_else(|| {
                        attributed_map_findings_in(&record.result.data, &BTreeMap::new(), universe)
                    }))
                }
                None => missing.push(call_id.clone()),
            }
        }
        let reduce_findings = reattribute(
            collect_findings(&result.data)
                .into_iter()
                .map(wrap_bare)
                .collect(),
            &map_findings,
        );
        let map_finding_count = map_findings.len();
        let reduce_finding_count = reduce_findings.len();
        let mut findings = merge_map_and_reduce(map_findings, reduce_findings);
        // Obs-31: a test red on the base commit in a file another task
        // declares was routed to that task by the branch whose filter found
        // it. The queue is drained into the FIRST mandated review's final
        // set — the one place the script reads findings for remediation —
        // stamped with the owner's id, so `remediateFindings` dispatches the
        // owner's write agent for it like any attributed finding.
        let baseline = if stage == REDUCE_FINAL_STAGE && kind == BASELINE_FINDINGS_REVIEW_KIND {
            baseline_findings_not_already_present(store, &findings)
        } else {
            Vec::new()
        };
        let baseline_finding_count = baseline.len();
        findings.extend(baseline);
        HostReviewFindings {
            kind,
            stage,
            findings,
            map_finding_count,
            reduce_finding_count,
            source_map_call_ids: sources,
            missing_source_map_call_ids: missing,
            baseline_finding_count,
        }
    };
    let mut attached_findings = attached_findings;
    attached_findings.findings = attached_findings
        .findings
        .into_iter()
        .map(|finding| normalize_task_ids_in(finding, universe))
        .collect();
    attach(&mut result.data, &attached_findings);
    Ok(())
}

#[path = "review_findings_sets.rs"]
mod sets;
pub use sets::{attached, attached_missing_sources, multiset, multiset_difference};

#[path = "review_findings_attribution.rs"]
mod attribution;
pub use attribution::{
    REFERENCED_IDS_KEY, normalize_task_ids, normalize_task_ids_in, stamp_task_ids, task_ids_of,
    wrap_bare,
};

#[cfg(test)]
#[path = "review_findings_tests.rs"]
mod tests;
