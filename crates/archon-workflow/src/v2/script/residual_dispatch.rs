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
        // A round's one read-only confirmation (`residual_confirm`) is
        // checked against the host's list of them, never as a round.
        if claimed.get("confirm").is_some() {
            return super::view::confirm::confirmation_refusal(
                execution,
                store,
                Some(universe),
                Some(root),
                key,
            );
        }
        let records = session_records(store);
        let refs: Vec<&WorkflowV2CallRecord> = records.iter().collect();
        let plan = plan_from(&refs, Some(universe), Some(root));
        // Issue-118: a round of the second pass is one of ITS plan, and says
        // so in its contract (its own records are then never its population).
        // Issue-121: and a round of the third pass one of the third's.
        let claims_later = claimed.get("pass").is_some();
        let later;
        let round = if claims_later {
            later = match claimed.get("pass").and_then(Value::as_u64) {
                Some(2) => super::second_pass_plan(&refs, store, Some(universe), Some(root)),
                Some(3) => super::third_pass_plan(&refs, store, Some(universe), Some(root)),
                // Batch O2: every later pass, while passes make progress.
                Some(pass) if pass >= 4 => {
                    super::later_pass_plan(pass, &refs, store, Some(universe), Some(root))
                }
                _ => return Some("its contract names no residual pass the host plans".into()),
            };
            later.rounds.iter().find(|round| round.key == key)
        } else {
            plan.rounds.iter().find(|round| round.key == key)
        };
        let Some(round) = round else {
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
        if round.kind == super::RoundKind::Adjudication
            && (call.write_mode.is_some() || call.method == WorkflowV2HostMethod::Checkpoint)
        {
            return Some("an adjudication is one read-only verifier".into());
        }
        if call.method == WorkflowV2HostMethod::Checkpoint {
            return None;
        }
        if items.len() != 1 {
            return Some("a round is exactly one item".into());
        }
        if let Some(missing) = unquoted(call, round, &store.load_call_records().unwrap_or_default())
        {
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
/// per gap, its id and its WHOLE description (m5) -- or, for a round an
/// earlier binary dispatched under the cut text, that cut text -- and for a
/// review round the unit. Ids and keys are read as their letters and digits;
/// a description is read exactly, JSON-quoted as the prelude quotes it.
fn unquoted(
    call: &super::super::WorkflowV2HostCall,
    round: &super::PlannedRound,
    stored: &[WorkflowV2CallRecord],
) -> Option<String> {
    let cut_ok = |residual: &super::Residual| {
        super::wording::dispatched_cut(stored, &round.key, &residual.description)
    };
    unquoted_in(
        call.options.task.as_deref().unwrap_or_default(),
        round,
        cut_ok,
    )
}

/// What of the plan `prompt` fails to carry; the check [`round_view`] runs
/// on the prompt the prelude will build, before anything is dispatched.
/// `cut_ok` says of a gap whether its cut description stands for it.
///
/// [`round_view`]: super::round_view
pub(super) fn unquoted_in(
    prompt: &str,
    round: &super::PlannedRound,
    cut_ok: impl Fn(&super::Residual) -> bool,
) -> Option<String> {
    let letters =
        |text: &str| -> String { text.chars().filter(char::is_ascii_alphanumeric).collect() };
    let plain = letters(prompt);
    if !plain.contains(letters(&round.key).as_str()) {
        return Some("key".to_string());
    }
    for residual in &round.residuals {
        if !plain.contains(letters(&residual.id).as_str()) {
            return Some(format!("gap `{}`", residual.id));
        }
        let cut = super::Wording::Legacy.description(&residual.description);
        let carried = super::wording::carries(prompt, &residual.description)
            || (cut_ok(residual)
                && super::wording::carries(prompt, cut.strip_suffix("...").unwrap_or(&cut)));
        if !carried {
            return Some(format!("text of gap `{}`", residual.id));
        }
    }
    if let Some(unit) = &round.unit_key
        && !plain.contains(letters(unit).as_str())
    {
        return Some("review unit".into());
    }
    None
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
