//! Issue-117: a residual round stands only on the host's own plan.
//!
//! The executed-plan validator sees a remediation contract; it cannot see
//! the plan that bought the round. So before a call claiming a residual
//! round is answered at all -- run or replayed -- the host rebuilds its plan
//! from this session's records (the round's own calls are never part of the
//! population, so asking changes nothing) and refuses the call unless its
//! key names a round of that plan not yet attempted, and its contract, tasks,
//! granted files and targets are exactly what the round allows. A write
//! item that names residual files without the contract is refused too: the
//! forbidden-path lift reads that field, and only a host plan may set it.
//! A refused call dispatches nothing and is not recorded; a refused fix reads
//! as a round that landed nothing, and the gap stands.

use std::collections::BTreeSet;
use std::path::Path;

use serde_json::{Value, json};

use super::super::{
    WorkflowV2CallExecution, WorkflowV2CallRecord, WorkflowV2HostMethod, WorkflowV2Result,
    WorkflowV2ResultStore, WorkflowV2Status, remediation_contract,
};
use super::{
    RESIDUAL_CONTRACT_KEY, RESIDUAL_ITEM_PATHS_KEY, done_checkpoint_id, plan_from, session_records,
};
use crate::task_universe::WorkflowV2TaskUniverse;
use crate::v2::script::remediation_escalation::unit_task_ids;
use crate::v2::script::residual_paths::owners;
use crate::v2::verification::path_ownership::{DeclaredPathForm, declared_path_form};

/// What the script is handed for a refused residual call: nothing landed.
pub fn refused_residual_result(reason: &str) -> WorkflowV2Result {
    WorkflowV2Result {
        status: WorkflowV2Status::Failed,
        summary: reason.to_string(),
        data: json!({ "patch_landed": false, "residual_refused": reason }),
        ..WorkflowV2Result::default()
    }
}

/// Why the call `execution` may not be answered as a residual round, or
/// `None` when it claims none or is exactly a round of the host's plan.
pub fn residual_refusal(
    execution: &WorkflowV2CallExecution,
    store: &WorkflowV2ResultStore,
    universe: Option<&WorkflowV2TaskUniverse>,
    repository_root: Option<&Path>,
) -> Option<String> {
    let call = &execution.call;
    let items: &[Value] = execution.input["source_data"]
        .as_array()
        .map(Vec::as_slice)
        .unwrap_or_default();
    let item = items.first().unwrap_or(&Value::Null);
    let claimed = remediation_contract(call).and_then(|c| c.get(RESIDUAL_CONTRACT_KEY));
    // Any item naming residual files, not only the first: the lift reads
    // each item's own field.
    let lifting = items
        .iter()
        .any(|item| item.get(RESIDUAL_ITEM_PATHS_KEY).is_some());
    let item_paths = item.get(RESIDUAL_ITEM_PATHS_KEY);
    if claimed.is_none() && !lifting {
        return None;
    }
    let why = (|| {
        let (Some(claimed), Some(contract)) = (claimed, remediation_contract(call)) else {
            return Some(
                "its item names residual files but its contract claims no host-planned round"
                    .to_string(),
            );
        };
        let (Some(universe), Some(root)) = (universe, repository_root) else {
            return Some("there is no task universe or repository root to check it against".into());
        };
        let key = claimed
            .get("key")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let records = session_records(store);
        let refs: Vec<&WorkflowV2CallRecord> = records.iter().collect();
        let plan = plan_from(&refs, Some(universe), Some(root));
        let Some(round) = plan.rounds.iter().find(|round| round.key == key) else {
            return Some(format!("no round of the host's plan is `{key}`"));
        };
        if store
            .load_call_record(&done_checkpoint_id(key))
            .ok()
            .flatten()
            .is_some()
        {
            return Some(format!("round `{key}` was already attempted in this run"));
        }
        let number = |field: &str| contract.get(field).and_then(Value::as_u64);
        if number("round") != Some(1)
            || number("maxRounds") != Some(1)
            || contract.get("escalation").is_some()
            || contract.get("reverify").is_some()
            || contract.get("contest").and_then(Value::as_str) != Some(key)
        {
            return Some("it is not the plan's one bounded round of its own unit".into());
        }
        if set(&claimed["files"]) != round.files {
            return Some(format!(
                "its files are not exactly the plan's ({:?})",
                round.files
            ));
        }
        if unit_task_ids(contract) != round.tasks {
            return Some(format!(
                "its tasks are not exactly the plan's ({:?})",
                round.tasks
            ));
        }
        if call.method == WorkflowV2HostMethod::Checkpoint {
            return None;
        }
        if items.len() != 1 {
            return Some("a round is exactly one item".into());
        }
        if let Some(missing) = unquoted(call, round) {
            return Some(format!("its prompt does not carry the plan's {missing}"));
        }
        if set(&item["canonical_task_ids"]) != round.tasks {
            return Some("its item does not name exactly the round's tasks".into());
        }
        if call.write_mode.is_none() {
            return item_paths
                .is_some()
                .then(|| "a verifier names no files to write".to_string());
        }
        if item_paths.map(set) != Some(round.files.clone()) {
            return Some("its item does not name exactly the round's granted files".into());
        }
        let mut targets = BTreeSet::new();
        for target in set(&item["target_files"]) {
            match declared_path_form(&target, root) {
                DeclaredPathForm::Repo(path) => targets.insert(path),
                _ => return Some(format!("its target {target} is no repository path")),
            };
        }
        if !round.files.is_subset(&targets) {
            return Some("its targets omit a granted file".into());
        }
        targets.difference(&round.files).find_map(|target| {
            owners(universe, target, root)
                .is_disjoint(&round.tasks)
                .then(|| format!("its target {target} is no declared file of the round's tasks"))
        })
    })()?;
    Some(format!("residual round `{}` refused: {why}", call.id))
}

/// What of the host's plan the call's prompt fails to carry: its key and,
/// per gap, the id and the opening of its text (for a review round, the
/// unit and the refused verdict). The prompt quotes them JSON-escaped, once
/// or twice, so both sides are read as their letters and digits only.
fn unquoted(
    call: &super::super::WorkflowV2HostCall,
    round: &super::PlannedRound,
) -> Option<String> {
    let letters =
        |text: &str| -> String { text.chars().filter(char::is_ascii_alphanumeric).collect() };
    let prompt = letters(call.options.task.as_deref().unwrap_or_default());
    let opening = |text: &str| {
        let head: String = text
            .chars()
            .take_while(|c| !c.is_control() && *c != '\\')
            .collect();
        letters(&head).chars().take(60).collect::<String>()
    };
    let mut needles = vec![("key".to_string(), letters(&round.key))];
    for residual in &round.residuals {
        needles.push((format!("gap `{}`", residual.id), letters(&residual.id)));
        needles.push((
            format!("text of gap `{}`", residual.id),
            opening(&residual.description),
        ));
    }
    if let Some(unit) = &round.unit_key {
        needles.push(("review unit".into(), letters(unit)));
    }
    needles
        .into_iter()
        .find(|(_, needle)| !prompt.contains(needle.as_str()))
        .map(|(what, _)| what)
}

fn set(value: &Value) -> BTreeSet<String> {
    value
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(str::to_string)
        .collect()
}
