//! ACC-A7: a frozen contract that falls short of its PRD is completed in the
//! round, never merely reported.
//!
//! `drift::drift_errors` finds what the contract owes the PRD as it is now:
//! an acceptance id with no check, and a requirement no check covers (every
//! contract frozen before `covers` existed covers none, so each requirement
//! is owed its `SUP-<id>` check). Each owed check is authored like any other
//! (`acceptance_author`), by the freeze-time judge the contract records, and
//! once nothing is owed they are added to the frozen chain through the
//! recorded republish (`republish::extend`). The round then runs them with
//! every other check. What is still owed stays a round error per check, so
//! authoring one more is progress; what the republish's gates refuse is a
//! round error naming it.

use std::collections::{BTreeMap, BTreeSet};

use archon_workflow::WorkflowResult;
use archon_workflow::obligation_ids::{acceptance_criteria, obligation_ids};
use archon_workflow::task_set_contract::{
    AcceptanceContract, AcceptanceCriterion, TASK_SKELETON_FILE, content_digest,
};
use archon_workflow::task_skeleton::TaskSkeleton;
use archon_workflow::v2::acceptance_stage::AcceptanceContractRepairV1;
use archon_workflow::v2::acceptance_stage::coverage::{
    missing_acceptance_ids, prd_requirement_texts, supplementary_requirement,
    uncovered_requirements,
};

use super::author::{
    Authored, REPAIR_TRIGGER_AUTHORED, Site, Staged, author_owed, owed_errors, owed_supplementary,
    placeholder, probe,
};
use crate::command::workflow_task_set::reauthor::ReauthorGate;
use crate::command::workflow_task_set::republish::ReauthorRequest;
use crate::command::workflow_task_set::republish::extend::{Extension, extend_and_republish};

/// The one (model, provider) every judged check of `contract` records.
fn recorded_judge(contract: &AcceptanceContract) -> Result<(String, String), String> {
    let judges: BTreeSet<(String, String)> = (contract.acceptance.iter())
        .chain(&contract.supplementary)
        .filter_map(|entry| {
            let sampling = entry.judgment.sampling.as_ref()?;
            Some((
                sampling["model"].as_str()?.to_string(),
                sampling["provider"].as_str()?.to_string(),
            ))
        })
        .collect();
    let mut judges = judges.into_iter();
    match (judges.next(), judges.next()) {
        (Some(judge), None) => Ok(judge),
        (None, _) => Err("the frozen contract records no judge to author its added checks with".into()),
        (Some(_), Some(_)) => Err(
            "the frozen contract records more than one judge, so no added check can be judged like the checks it keeps".into(),
        ),
    }
}

/// Complete `contract` against its PRD as it is now (see the module doc).
/// `None` when it owes nothing, or its PRD cannot be read (the drift errors
/// report that).
pub(super) async fn heal_drift(
    site: &Site<'_>,
    contract: &AcceptanceContract,
) -> WorkflowResult<Option<Authored>> {
    let context = site.context;
    let prd_path = super::drift::prd_path(context, contract);
    let Ok(prd_text) = std::fs::read_to_string(&prd_path) else {
        return Ok(None);
    };
    let prd_digest = content_digest(prd_text.as_bytes());
    let owed = owed_by(contract, &prd_text, prd_digest != contract.prd.digest);
    if owed.is_empty() {
        return Ok(None);
    }
    let how = "the frozen contract falls short of its PRD";
    let blocked = |why: String| {
        Ok(Some(Authored {
            repair: None,
            errors: vec![format!(
                "the frozen contract owes {} check(s) its PRD requires ({}), and the host cannot author them: {why}",
                owed.len(),
                owed.iter()
                    .map(|e| e.id.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            )],
        }))
    };
    let (model, provider) = match recorded_judge(contract) {
        Ok(judge) => judge,
        Err(why) => return blocked(why),
    };
    if let Some(llm) = site.llm
        && llm.provider_id().as_deref() != Some(provider.as_str())
    {
        return blocked(format!(
            "the freeze-time judge was {model} on provider {provider}, but this run's author client serves {}",
            llm.provider_id()
                .as_deref()
                .unwrap_or("an unreported provider")
        ));
    }
    let mut staged = match Staged::load(site.run_dir, &prd_digest) {
        Ok(staged) => staged,
        Err(why) => return blocked(why),
    };
    let diagnostics =
        match author_owed(site, &prd_path, contract, &owed, &model, &mut staged).await? {
            Ok(diagnostics) => diagnostics,
            Err(why) => return blocked(why),
        };
    let errors = owed_errors(&owed, &staged, how);
    if !errors.is_empty() {
        return Ok(Some(Authored {
            repair: None,
            errors,
        }));
    }
    let entries: Vec<_> = (owed.iter())
        .filter_map(|entry| staged.entries.get(&entry.id).cloned())
        .collect();
    let ids: BTreeSet<String> = entries.iter().map(|entry| entry.id.clone()).collect();
    let extension = Extension {
        entries,
        prd_digest: prd_digest.clone(),
        frozen_prd_digest: contract.prd.digest.clone(),
        new_obligations: match unclaimed(context, contract, &prd_text, &prd_digest) {
            Ok(unclaimed) => unclaimed,
            Err(why) => return blocked(why),
        },
    };
    let probe = match probe(site) {
        Ok(probe) => probe,
        Err(why) => return blocked(why),
    };
    let seeds = Default::default();
    let published = extend_and_republish(
        ReauthorRequest {
            project_root: &context.project,
            tasks_root: &context.task_root,
            prd_path: &prd_path,
            ids: &ids,
            gate: ReauthorGate {
                probe: &probe,
                seeds: &seeds,
            },
            trigger: "in-round acceptance extension (the frozen contract fell short of its PRD)",
        },
        &extension,
    );
    let mut repair = AcceptanceContractRepairV1 {
        check_ids: ids.iter().cloned().collect(),
        trigger: REPAIR_TRIGGER_AUTHORED.into(),
        repaired: false,
        freeze_event_id: String::new(),
        failure: String::new(),
        diagnostics,
    };
    Ok(Some(match published {
        Ok(result) => {
            repair.repaired = true;
            repair.freeze_event_id = result.freeze_event_id;
            repair.diagnostics.extend(result.diagnostics);
            repair.diagnostics.extend(Staged::clear(site.run_dir));
            Authored {
                repair: Some(repair),
                errors: Vec::new(),
            }
        }
        Err(error) => {
            repair.failure = format!("{error:#}");
            Authored {
                errors: vec![format!(
                    "the host authored every check the frozen contract owes its PRD ({}), but the republish that adds them was refused: {error:#}",
                    ids.iter().cloned().collect::<Vec<_>>().join(", ")
                )],
                repair: Some(repair),
            }
        }
    }))
}

/// Obligations of the PRD as it is now that no skeleton task claims, when
/// the PRD moved since the freeze; empty when it did not or there is no
/// skeleton.
fn unclaimed(
    context: &super::exec::StageContext,
    contract: &AcceptanceContract,
    prd_text: &str,
    prd_digest: &str,
) -> Result<BTreeSet<String>, String> {
    if prd_digest == contract.prd.digest {
        return Ok(BTreeSet::new());
    }
    let path = context.task_root.join(TASK_SKELETON_FILE);
    let bytes = match std::fs::read(&path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(BTreeSet::new()),
        Err(error) => {
            return Err(format!(
                "the frozen skeleton {} cannot be read: {error}",
                path.display()
            ));
        }
    };
    let skeleton: TaskSkeleton = serde_json::from_slice(&bytes).map_err(|error| {
        format!(
            "the frozen skeleton {} is unreadable: {error}",
            path.display()
        )
    })?;
    let claimed: BTreeSet<&String> = (skeleton.tasks.iter())
        .flat_map(|task| &task.implements)
        .collect();
    Ok(obligation_ids(prd_text)
        .into_iter()
        .filter(|id| !claimed.contains(id))
        .collect())
}

/// Every check the contract owes its PRD as it is now, as a placeholder to
/// author (M3/M4): an acceptance id with no check; a requirement no check
/// covers, and a supplementary check that does not cover the requirement
/// it is owed for -- re-authored, never dropped; and, when the PRD moved
/// since the freeze, every entry whose criterion is no longer the PRD's
/// text or whose `covers` names an id the PRD no longer defines.
pub(super) fn owed_by(
    contract: &AcceptanceContract,
    prd_text: &str,
    prd_moved: bool,
) -> Vec<AcceptanceCriterion> {
    let criteria = acceptance_criteria(prd_text);
    let texts = prd_requirement_texts(prd_text);
    let requirements: BTreeSet<String> = texts.keys().cloned().collect();
    let mut owed: BTreeMap<String, AcceptanceCriterion> = BTreeMap::new();
    for id in missing_acceptance_ids(&criteria.keys().cloned().collect(), contract) {
        owed.insert(id.clone(), placeholder(&id, &criteria[&id], Vec::new()));
    }
    let uncovered: BTreeSet<String> = uncovered_requirements(&requirements, contract)
        .into_iter()
        .collect();
    for sup in owed_supplementary(prd_text, &uncovered) {
        owed.insert(sup.id.clone(), sup);
    }
    for entry in contract.acceptance.iter().chain(&contract.supplementary) {
        let current_covers: Vec<String> = (entry.covers.iter())
            .filter(|id| requirements.contains(id.trim()))
            .cloned()
            .collect();
        let stale = match supplementary_requirement(&entry.id) {
            // A supplementary check is owed for exactly its requirement.
            Some(requirement) => {
                let Some(text) = texts.get(requirement) else {
                    continue;
                };
                let covers = entry.covers.iter().any(|id| id.trim() == requirement);
                (!covers
                    || (prd_moved
                        && (entry.criterion != *text
                            || current_covers.len() != entry.covers.len())))
                .then(|| placeholder(&entry.id, text, vec![requirement.to_string()]))
            }
            None => {
                let Some(text) = criteria.get(&entry.id) else {
                    continue;
                };
                (prd_moved
                    && (entry.criterion != *text || current_covers.len() != entry.covers.len()))
                .then(|| placeholder(&entry.id, text, current_covers))
            }
        };
        if let Some(stale) = stale {
            owed.insert(stale.id.clone(), stale);
        }
    }
    owed.into_values().collect()
}
