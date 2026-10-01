//! REM-13 / ACC-A7: acceptance checks the host authors mid-run.
//!
//! Two holes left a round with nothing it could run, and the run blocked on
//! a note: a task set with NO frozen contract (a pre-rule script, or a set
//! never frozen), and a frozen contract that falls short of its PRD (an
//! acceptance id with no check, or a requirement no check covers -- every
//! contract frozen before `covers` existed covers none). Both are healed
//! here, at the stage's entry, with the freeze's own machinery:
//!
//! - each owed check starts as a host placeholder (its id, the PRD's exact
//!   criterion or requirement text, its `covers`; never publishable) and is
//!   authored by the freeze's bounded re-author (`reauthor::reauthor`):
//!   authored against the PRD and the repository, judged adversarially,
//!   run once to prove it does not crash, and run on the run's base commit
//!   to prove it can FAIL (A5);
//! - every accepted entry is staged under the run
//!   (`v2/acceptance-authoring.json`) the moment it is accepted, so a round
//!   that could not author everything keeps what it did, and the next round
//!   authors only what is still owed: the loop follows progress (each owed
//!   check still unauthored is a round error of its own, so authoring one
//!   more IS progress), never a count;
//! - once nothing is owed, the whole set is published: a fresh freeze
//!   through the enforce gate (this module), or an extension of the frozen
//!   chain through the recorded republish (`acceptance_author_drift`).
//!
//! Nothing here knows a PRD, a language or a domain: ids, texts and paths
//! come from the PRD, the task set and the run.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use archon_workflow::task_set_contract::{
    ACCEPTANCE_LOCK_FILE, AcceptanceCheck, AcceptanceContract, AcceptanceCriterion, GapPolicy,
    JudgeDecision, JudgeVerdict, PrdIdentity, REQUIRED_RESIDUAL_GAP_FIELDS, TrustedCwd,
};
use archon_workflow::v2::acceptance_stage::AcceptanceContractRepairV1;
use archon_workflow::v2::acceptance_stage::coverage::{
    prd_requirement_texts, supplementary_id, supplementary_requirement,
};
use archon_workflow::{WorkflowLlmClient, WorkflowResult, WorkflowStore, poll_v2_run_control};

use super::exec::StageContext;
use crate::command::workflow_task_set::executability::{
    Baseline, ExecutabilityProbe, HostProbe, PLACEHOLDER_REASON,
};
use crate::command::workflow_task_set::reauthor::{AuthorScope, ReauthorGate, reauthor};

#[path = "workflow_live_v3_acceptance_author_prd.rs"]
mod prd;
#[path = "workflow_live_v3_acceptance_author_publish.rs"]
mod publish;
#[path = "workflow_live_v3_acceptance_author_staging.rs"]
mod staging;
use publish::{Refused, publish_fresh};
pub(super) use staging::Staged;
#[cfg(test)]
use staging::staging_path;

/// Why a repair record says the host authored checks.
pub(super) const REPAIR_TRIGGER_AUTHORED: &str = "authored_in_run";
/// The judge a fresh in-run freeze authors against: the whole-set freeze's.
const FRESH_JUDGE_MODEL: &str = "sonnet";

/// Where the round authors: its roots, the author client, the base commit
/// every authored check must fail on, and the run control it polls.
pub(super) struct Site<'a> {
    pub(super) llm: Option<&'a dyn WorkflowLlmClient>,
    pub(super) context: &'a StageContext,
    pub(super) run_dir: &'a Path,
    pub(super) base: Option<&'a str>,
    pub(super) store: &'a WorkflowStore,
    pub(super) run_id: &'a str,
    pub(super) call_id: &'a str,
}

/// What an authoring attempt left the round.
#[derive(Default)]
pub(super) struct Authored {
    /// The repair record, when anything was published or failed to be.
    pub(super) repair: Option<AcceptanceContractRepairV1>,
    /// One round error per check still owed, and any reason nothing could
    /// be authored: each blocks completion by name.
    pub(super) errors: Vec<String>,
}

/// A host placeholder for an owed check: never publishable (the judge has
/// not accepted it), only the entry the author replaces.
pub(super) fn placeholder(id: &str, criterion: &str, covers: Vec<String>) -> AcceptanceCriterion {
    AcceptanceCriterion {
        id: id.to_string(),
        criterion: criterion.to_string(),
        check: AcceptanceCheck::Command {
            command: "false".into(),
            cwd: TrustedCwd::RepoRoot,
        },
        gap_permitted: false,
        judgment: JudgeVerdict {
            verdict: JudgeDecision::Refuted,
            // The executability probe's marker: a placeholder is never a
            // strength baseline for what replaces it.
            counterexample: "author a check that exercises the deliverable and fails whenever the criterion is false".into(),
            reason: PLACEHOLDER_REASON.into(),
            // The probe validates the whole working contract, every entry
            // carrying the id of the judgment that placed it.
            host_call_id: format!("host-placeholder:{id}"),
            sampling: None,
        },
        covers,
    }
}

/// The owed supplementary check of each requirement in `requirements`.
pub(super) fn owed_supplementary(
    prd_text: &str,
    requirements: &BTreeSet<String>,
) -> Vec<AcceptanceCriterion> {
    let texts = prd_requirement_texts(prd_text);
    (requirements.iter())
        .map(|requirement| {
            let text = texts.get(requirement).cloned().unwrap_or_default();
            placeholder(
                &supplementary_id(requirement),
                &text,
                vec![requirement.clone()],
            )
        })
        .collect()
}

/// `base` with every owed entry present: staged ones as authored, the rest
/// as placeholders.
fn working(
    base: &AcceptanceContract,
    owed: &[AcceptanceCriterion],
    staged: &Staged,
) -> AcceptanceContract {
    let mut contract = base.clone();
    for entry in owed {
        let entry = staged.entries.get(&entry.id).unwrap_or(entry).clone();
        let list = if supplementary_requirement(&entry.id).is_some() {
            &mut contract.supplementary
        } else {
            &mut contract.acceptance
        };
        match list.iter_mut().find(|held| held.id == entry.id) {
            Some(held) => *held = entry,
            None => list.push(entry),
        }
    }
    contract.acceptance.sort_by(|a, b| a.id.cmp(&b.id));
    contract.supplementary.sort_by(|a, b| a.id.cmp(&b.id));
    contract
}

/// The probe every authored check clears: it runs without crashing at the
/// round's own site, and it fails on the tree before implementation -- the
/// run's base commit, else the commit the task set's decomposition recorded
/// (`Baseline::for_task_set`). With neither, no check can be proven able to
/// fail, so none is authored (M5): an error, never an unproven publish.
pub(super) fn probe(site: &Site<'_>) -> Result<HostProbe, String> {
    let context = site.context;
    let baseline = match site.base {
        Some(commit) => Baseline {
            commit: commit.to_string(),
            repository: context.repository.clone(),
        },
        None => Baseline::for_task_set(&context.repository, &context.task_root).ok_or_else(|| {
            format!(
                "no pre-implementation tree is known to prove an authored check can fail: the run recorded no base commit and {} is not a git checkout the task set records",
                context.repository.display()
            )
        })?,
    };
    Ok(HostProbe::at(
        context.project.clone(),
        context.repository.clone(),
        context.binding.clone(),
    )
    .with_baseline(baseline))
}

/// Author every owed entry not yet staged, one at a time, staging each the
/// moment it is accepted. A pause or cancel between entries unwinds with
/// everything authored so far kept. Returns the probe's diagnostics.
pub(super) async fn author_owed(
    site: &Site<'_>,
    prd_path: &Path,
    base: &AcceptanceContract,
    owed: &[AcceptanceCriterion],
    judge_model: &str,
    staged: &mut Staged,
) -> WorkflowResult<Result<Vec<String>, String>> {
    let Some(llm) = site.llm else {
        return Ok(Err(
            "this acceptance stage has no author client to author checks with".into(),
        ));
    };
    let scope = AuthorScope {
        prd_path: prd_path.to_path_buf(),
        project_root: site.context.project.clone(),
        repository_root: site.context.repository.clone(),
    };
    let probe = match probe(site) {
        Ok(probe) => probe,
        Err(why) => return Ok(Err(why)),
    };
    let pending: Vec<String> = (owed.iter())
        .map(|entry| entry.id.clone())
        .filter(|id| !staged.entries.contains_key(id))
        .collect();
    for id in pending {
        poll_v2_run_control(site.store, site.run_id, site.call_id)?;
        let contract = working(base, owed, staged);
        let seeds: BTreeMap<String, String> = (staged.feedback.get(&id))
            .map(|seed| BTreeMap::from([(id.clone(), seed.clone())]))
            .unwrap_or_default();
        let gate = ReauthorGate {
            probe: &probe,
            seeds: &seeds,
        };
        let ids = BTreeSet::from([id.clone()]);
        match reauthor(llm, &contract, &ids, &scope, judge_model, &gate).await {
            Ok(repaired) => {
                let entry = (repaired.acceptance.iter())
                    .chain(&repaired.supplementary)
                    .find(|entry| {
                        entry.id == id && entry.judgment.verdict == JudgeDecision::Accepted
                    })
                    .cloned();
                match entry {
                    Some(entry) => {
                        staged.feedback.remove(&id);
                        staged.entries.insert(id.clone(), entry);
                    }
                    None => staged.reject(&id, "the author returned no accepted entry"),
                }
            }
            Err(error) => staged.reject(&id, &format!("{error:#}")),
        }
        if let Err(why) = staged.save(site.run_dir) {
            return Ok(Err(why));
        }
    }
    Ok(Ok(probe.take_diagnostics()))
}

/// One round error per owed check still unauthored, with its last finding.
pub(super) fn owed_errors(owed: &[AcceptanceCriterion], staged: &Staged, how: &str) -> Vec<String> {
    (owed.iter())
        .filter(|entry| !staged.entries.contains_key(&entry.id))
        .map(|entry| {
            format!(
                "acceptance check {} is owed ({how}) and not yet authored and accepted; the host authors it again next round{}",
                entry.id,
                staged
                    .feedback
                    .get(&entry.id)
                    .map_or(String::new(), |why| format!(": {why}"))
            )
        })
        .collect()
}

fn project_relative(root: &Path, path: &Path) -> String {
    let root = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
    let path = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    path.strip_prefix(&root)
        .unwrap_or(&path)
        .to_string_lossy()
        .replace('\\', "/")
}

/// REM-13: a task set with no frozen contract -- none at all, or an
/// unfrozen candidate, and no pin -- gets one authored and frozen here.
/// `None` when the task set is frozen (or its chain is only partly there,
/// which is the stage's integrity error, never a reason to re-freeze).
pub(super) async fn heal_unfrozen(site: &Site<'_>) -> WorkflowResult<Option<Authored>> {
    let context = site.context;
    let pin = crate::command::workflow_task_set::acceptance_pin_path(
        &context.project,
        &context.task_root,
    );
    if context.task_root.join(ACCEPTANCE_LOCK_FILE).exists() || pin.exists() {
        return Ok(None);
    }
    let failed = |why: String| {
        Ok(Some(Authored {
            repair: None,
            errors: vec![format!(
                "{} holds no frozen acceptance contract, and the host could not author one: {why}",
                context.task_root.display()
            )],
        }))
    };
    // An unfrozen candidate names its PRD; otherwise the task set's
    // decomposition recorded it. A candidate that cannot be read names
    // nothing the host may trust: never a guess.
    let named = match std::fs::read(context.contract_path()) {
        Ok(bytes) => match serde_json::from_slice::<AcceptanceContract>(&bytes) {
            Ok(candidate) => Some(super::drift::prd_path(context, &candidate)),
            Err(error) => {
                return failed(format!(
                    "the unfrozen candidate {} is not an acceptance contract ({error}), so the PRD it answers to is not recorded",
                    context.contract_path().display()
                ));
            }
        },
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(error) => {
            return failed(format!(
                "the unfrozen candidate {} cannot be read: {error}",
                context.contract_path().display()
            ));
        }
    };
    // The run store this run lives in holds every decomposition record.
    let Some(runs) = site.run_dir.parent() else {
        return failed(
            "the run directory has no run store to read decomposition records from".into(),
        );
    };
    let prd_path = match prd::recorded(runs, &context.task_root, named) {
        Ok(path) => path,
        Err(why) => return failed(why),
    };
    let (prd_bytes, prd_digest, criteria) =
        match crate::command::workflow_task_set::validate_prd_input(&prd_path) {
            Ok(read) => read,
            Err(error) => return failed(format!("PRD {}: {error:#}", prd_path.display())),
        };
    let prd_text = String::from_utf8_lossy(&prd_bytes).into_owned();
    let mut owed: Vec<AcceptanceCriterion> = (criteria.iter())
        .map(|(id, text)| placeholder(id, text, Vec::new()))
        .collect();
    let requirements = prd_requirement_texts(&prd_text).into_keys().collect();
    owed.extend(owed_supplementary(&prd_text, &requirements));
    let base = AcceptanceContract {
        schema_version: 1,
        prd: PrdIdentity {
            path: project_relative(&context.project, &prd_path),
            digest: prd_digest.clone(),
        },
        gap_policy: GapPolicy {
            permitted_acceptance_ids: BTreeSet::new(),
            forbidden_phrases: archon_workflow::obligation_ids::residual_gap_forbidden_phrases(
                &prd_text,
            ),
            required_fields: REQUIRED_RESIDUAL_GAP_FIELDS
                .iter()
                .map(|field| (*field).to_string())
                .collect(),
        },
        acceptance: Vec::new(),
        supplementary: Vec::new(),
    };
    let mut staged = match Staged::load(site.run_dir, &prd_digest) {
        Ok(staged) => staged,
        Err(why) => return failed(why),
    };
    let diagnostics = match author_owed(
        site,
        &prd_path,
        &base,
        &owed,
        FRESH_JUDGE_MODEL,
        &mut staged,
    )
    .await?
    {
        Ok(diagnostics) => diagnostics,
        Err(why) => return failed(why),
    };
    let how = "the task set has no frozen contract";
    let errors = owed_errors(&owed, &staged, how);
    let check_ids: Vec<String> = owed.iter().map(|entry| entry.id.clone()).collect();
    if !errors.is_empty() {
        return Ok(Some(Authored {
            repair: None,
            errors,
        }));
    }
    let contract = working(&base, &owed, &staged);
    let published = publish_fresh(context, &prd_path, &contract);
    let mut repair = AcceptanceContractRepairV1 {
        check_ids,
        trigger: REPAIR_TRIGGER_AUTHORED.into(),
        repaired: false,
        freeze_event_id: String::new(),
        failure: String::new(),
        diagnostics,
    };
    match published {
        Ok(event) => {
            repair.repaired = true;
            repair.freeze_event_id = event;
            repair.diagnostics.extend(Staged::clear(site.run_dir));
            Ok(Some(Authored {
                repair: Some(repair),
                errors: Vec::new(),
            }))
        }
        Err(Refused { findings, error }) => {
            // Each check a gate finding names goes back to its author.
            for (subject, text) in &findings {
                if staged.entries.contains_key(subject) {
                    staged.reject(subject, text);
                }
            }
            repair.failure = error.clone();
            let mut errors = owed_errors(&owed, &staged, how);
            errors.extend(staged.save(site.run_dir).err());
            if errors.is_empty() {
                errors.push(format!(
                    "the host authored every owed acceptance check, but the freeze gate refused the contract: {error}"
                ));
            }
            Ok(Some(Authored {
                repair: Some(repair),
                errors,
            }))
        }
    }
}

#[cfg(test)]
#[path = "workflow_live_v3_acceptance_author_tests.rs"]
mod tests;
