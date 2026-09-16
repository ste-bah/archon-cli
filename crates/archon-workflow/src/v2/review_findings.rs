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
use crate::{WorkflowError, WorkflowResult};

/// Arrays that carry findings, wherever they sit in an envelope.
pub const FINDINGS_ARRAY_KEYS: [&str; 3] = ["findings", "adversarial_findings", "uncovered_requirements"];

/// Fields that identify a finding; sharing any one of them is the same finding.
pub const IDENTITY_KEYS: [&str; 6] = ["id", "title", "claim", "summary", "finding", "requirement_id"];

/// The `data` field under which the host attaches the review finding set.
pub const HOST_REVIEW_FINDINGS_KEY: &str = "review_findings";

/// Marker on a reduce finding the maps never saw.
pub const CROSS_CUTTING_SCOPE: &str = "cross_cutting";

const REVIEW_CONTRACT_KEYS: [&str; 2] = ["reviewContract", "review_contract"];
const SOURCE_MAP_KEYS: [&str; 2] = ["sourceMapCallIds", "source_map_call_ids"];
const MAP_STAGE: &str = "map";

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
            (!text.is_empty()).then(|| format!("{key}:{}", text.chars().take(200).collect::<String>()))
        })
        .collect()
}

/// A stable key for multiset comparison: the identities, or the whole document
/// for a finding that has none.
pub fn finding_key(finding: &Value) -> String {
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

/// The task ids a value declares, under any of the spellings agents use.
pub fn task_ids_of(value: &Value) -> Vec<String> {
    let Some(object) = value.as_object() else {
        return Vec::new();
    };
    for key in ["canonical_task_ids", "task_ids", "taskIds", "task_id"] {
        let ids = match object.get(key) {
            Some(Value::Array(items)) => items
                .iter()
                .filter_map(Value::as_str)
                .map(str::trim)
                .filter(|id| !id.is_empty())
                .map(str::to_string)
                .collect::<Vec<_>>(),
            Some(Value::String(id)) if !id.trim().is_empty() => vec![id.trim().to_string()],
            _ => Vec::new(),
        };
        if !ids.is_empty() {
            return ids;
        }
    }
    Vec::new()
}

/// Stamp `canonical_task_ids` onto a finding that declares none. Attribution a
/// reviewer supplied itself is never overwritten -- a finding that legitimately
/// names several tasks keeps all of them -- and a non-object finding (a bare
/// requirement id, say) is returned untouched rather than shredded.
pub fn stamp_task_ids(finding: Value, task_ids: &[String]) -> Value {
    let Value::Object(mut object) = finding else {
        return finding;
    };
    if task_ids.is_empty() || !task_ids_of(&Value::Object(object.clone())).is_empty() {
        return Value::Object(object);
    }
    object.insert("canonical_task_ids".to_string(), Value::from(task_ids.to_vec()));
    Value::Object(object)
}

/// The findings of a review map, each stamped with the task its branch
/// reviewed. A branch's task comes from what the branch itself reported, then
/// from the input the host built for it; a finding the host cannot place is
/// still returned, unattributed, rather than dropped.
pub fn attributed_map_findings(
    map_result_data: &Value,
    item_task_ids: &BTreeMap<String, Vec<String>>,
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
        collected.extend(branch.into_iter().map(|finding| stamp_task_ids(finding, &task_ids)));
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
                object.insert("finding_scope".to_string(), Value::from(CROSS_CUTTING_SCOPE));
                Value::Object(object)
            }
            other => other,
        });
    }
    merged
}

/// Count of each finding by key.
pub fn multiset(findings: &[Value]) -> BTreeMap<String, usize> {
    let mut counts = BTreeMap::new();
    for finding in findings {
        *counts.entry(finding_key(finding)).or_default() += 1;
    }
    counts
}

/// Findings in `have` that `want` lacks (by key, honouring multiplicity).
pub fn multiset_difference(have: &[Value], want: &[Value]) -> Vec<Value> {
    let mut remaining = multiset(want);
    have.iter()
        .filter(|finding| {
            let key = finding_key(finding);
            match remaining.get_mut(&key) {
                Some(count) if *count > 0 => {
                    *count -= 1;
                    false
                }
                _ => true,
            }
        })
        .cloned()
        .collect()
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
        })
    }
}

/// The host-attached finding set on a result's `data`, if any.
pub fn attached(data: &Value) -> Option<Vec<Value>> {
    data.get(HOST_REVIEW_FINDINGS_KEY)?
        .get("findings")?
        .as_array()
        .cloned()
}

/// The source maps a reduce named whose results were missing when it ran.
pub fn attached_missing_sources(data: &Value) -> Vec<String> {
    data.get(HOST_REVIEW_FINDINGS_KEY)
        .and_then(|value| value.get("missing_source_map_call_ids"))
        .and_then(Value::as_array)
        .map(|ids| ids.iter().filter_map(Value::as_str).map(str::to_string).collect())
        .unwrap_or_default()
}

fn attach(data: &mut Value, findings: &HostReviewFindings) {
    if !data.is_object() {
        *data = Value::Object(Map::new());
    }
    if let Some(object) = data.as_object_mut() {
        object.insert(HOST_REVIEW_FINDINGS_KEY.to_string(), findings.to_value());
    }
}

/// The review contract a call carries, under either spelling. Shared with
/// `review_roster`, which reads the same contract before the call runs.
pub(crate) fn review_contract(execution: &WorkflowV2CallExecution) -> Option<&Map<String, Value>> {
    REVIEW_CONTRACT_KEYS
        .iter()
        .find_map(|key| execution.call.options.extra.get(*key))
        .and_then(Value::as_object)
}

fn contract_string(contract: &Map<String, Value>, key: &str) -> String {
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
            let declared = item
                .input
                .get("item")
                .map(task_ids_of)
                .unwrap_or_default();
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
        let findings = attributed_map_findings(&result.data, &item_task_ids(execution, store)?);
        HostReviewFindings {
            kind,
            stage,
            map_finding_count: findings.len(),
            reduce_finding_count: 0,
            findings,
            source_map_call_ids: Vec::new(),
            missing_source_map_call_ids: Vec::new(),
        }
    } else {
        let sources = source_map_call_ids(contract);
        let mut map_findings = Vec::new();
        let mut missing = Vec::new();
        for call_id in &sources {
            match store.load_call_record(call_id)? {
                Some(record) => map_findings.extend(
                    attached(&record.result.data)
                        .unwrap_or_else(|| attributed_map_findings(&record.result.data, &BTreeMap::new())),
                ),
                None => missing.push(call_id.clone()),
            }
        }
        let reduce_findings = reattribute(collect_findings(&result.data), &map_findings);
        let map_finding_count = map_findings.len();
        let reduce_finding_count = reduce_findings.len();
        HostReviewFindings {
            kind,
            stage,
            findings: merge_map_and_reduce(map_findings, reduce_findings),
            map_finding_count,
            reduce_finding_count,
            source_map_call_ids: sources,
            missing_source_map_call_ids: missing,
        }
    };
    attach(&mut result.data, &attached_findings);
    Ok(())
}

#[cfg(test)]
#[path = "review_findings_tests.rs"]
mod tests;
