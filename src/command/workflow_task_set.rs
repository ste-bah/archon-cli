//! Host-owned preparation and atomic publication of decomposition freezes.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context, Result, anyhow};
use archon_core::config::GateMode;
use archon_workflow::llm_client_port::WorkflowLlmClient;
use archon_workflow::obligation_ids::{
    acceptance_criteria, duplicate_obligation_finding, duplicate_obligation_ids,
    malformed_obligation_finding, malformed_obligation_ids, residual_gap_forbidden_phrases,
};
use archon_workflow::task_set_contract::{
    ACCEPTANCE_CONTRACT_FILE, AcceptanceContract, AcceptanceLock, AcceptancePin, FreezeGateMode,
    REQUIRED_RESIDUAL_GAP_FIELDS, TASK_SKELETON_FILE, acceptance_policy_findings, content_digest,
    criterion_prescribes_check_shape, validate_acceptance_bundle, validate_acceptance_structure,
};
use archon_workflow::task_set_edges::analyze_task_set_edges;
use archon_workflow::task_skeleton::{
    TaskSkeleton, TaskSkeletonLock, validate_skeleton, validate_skeleton_set,
};

use crate::command::workflow_gate::{GateFinding, GateId};

#[path = "workflow_acceptance_coverage_gate.rs"]
mod coverage_gate;
#[path = "workflow_task_set_findings.rs"]
mod findings;
#[path = "workflow_judge_incremental.rs"]
mod incremental;
#[path = "workflow_task_set_judge.rs"]
pub(crate) mod judge;
#[path = "workflow_judge_store.rs"]
mod judge_store;
#[path = "workflow_task_set_merge.rs"]
mod merge;
#[path = "workflow_acceptance_passability.rs"]
pub(crate) mod passability;
#[path = "workflow_acceptance_preflight.rs"]
mod preflight;
use judge::{gate_stamp, judge_contract, predecessor_findings};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct FreezeAcceptanceResult {
    pub(crate) acceptance_digest: String,
    pub(crate) freeze_event_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct FreezeSkeletonResult {
    pub(crate) skeleton_digest: String,
    pub(crate) acceptance_digest: String,
}

#[derive(Debug)]
pub(crate) struct PreparedAcceptanceFreeze {
    tasks_root: PathBuf,
    project_root: PathBuf,
    contract_bytes: Vec<u8>,
    lock: AcceptanceLock,
    pin: AcceptancePin,
    recovery: Option<recovery_lineage::Publication>,
    /// Checks the judge did not accept. Never published: such a check can
    /// never run, so a freeze carrying one is refused at publication.
    non_accepted: BTreeSet<String>,
    pub(crate) findings: Vec<GateFinding>,
    pub(crate) result: FreezeAcceptanceResult,
}

#[derive(Debug)]
pub(crate) struct PreparedSkeletonFreeze {
    tasks_root: PathBuf,
    pin_path: PathBuf,
    skeleton_bytes: Vec<u8>,
    lock: TaskSkeletonLock,
    pin: AcceptancePin,
    pub(crate) findings: Vec<GateFinding>,
    pub(crate) result: FreezeSkeletonResult,
}

#[path = "workflow_task_set_recovery_lineage.rs"]
pub(crate) mod recovery_lineage;

#[path = "workflow_task_set_enforce.rs"]
mod enforce;
#[path = "workflow_task_set_identity.rs"]
mod identity;
#[path = "workflow_task_set_prd.rs"]
mod prd;
#[path = "workflow_task_set_staging.rs"]
mod staging;
pub(crate) use crate::command::workflow_task_set_candidate::CandidateRejected;
#[cfg(test)]
pub(crate) use enforce::{freeze_acceptance, freeze_skeleton};
pub(crate) use prd::validate_prd_input;

pub(crate) fn acceptance_pin_path(project_root: &Path, tasks_root: &Path) -> PathBuf {
    let canonical = tasks_root
        .canonicalize()
        .map(archon_shell::paths::plain)
        .unwrap_or_else(|_| tasks_root.to_path_buf());
    let key = content_digest(canonical.to_string_lossy().as_bytes());
    archon_workflow::task_set_lineage::pin_store_dir(project_root).join(format!("{key}.json"))
}

pub(crate) async fn prepare_acceptance_freeze(
    project_root: &Path,
    tasks_root: &Path,
    prd_path: &Path,
    mode: GateMode,
    client: Arc<dyn WorkflowLlmClient>,
) -> Result<PreparedAcceptanceFreeze> {
    let contract_path = tasks_root.join(ACCEPTANCE_CONTRACT_FILE);
    let candidate = std::fs::read(&contract_path)
        .with_context(|| format!("reading acceptance contract at {}", contract_path.display()))?;
    prepare_acceptance_freeze_from_candidate(
        project_root,
        tasks_root,
        prd_path,
        mode,
        candidate,
        client,
    )
    .await
}

/// The unstaged freeze of a candidate: unlimited, nothing saved.
pub(crate) async fn prepare_acceptance_freeze_from_candidate(
    project_root: &Path,
    tasks_root: &Path,
    prd_path: &Path,
    mode: GateMode,
    candidate: Vec<u8>,
    client: Arc<dyn WorkflowLlmClient>,
) -> Result<PreparedAcceptanceFreeze> {
    let resume = crate::command::workflow_freeze_budget::FreezeResume::none();
    prepare_acceptance_freeze_resumable(
        project_root,
        tasks_root,
        prd_path,
        mode,
        candidate,
        client,
        &resume,
    )
    .await
}

/// [`prepare_acceptance_freeze_from_candidate`] under `resume` (Issue 255):
/// judge verdicts and probe verdicts saved for a retry, and a budget that
/// ends the freeze `FreezeIncomplete` rather than at the host's kill.
/// Issue 260: under a staged freeze, a judge that could not complete is an
/// incomplete, resumable freeze (the executor retries it while progress
/// grows, then pauses the run), never an operational failure that ends it.
fn resumable(
    error: anyhow::Error,
    resume: &crate::command::workflow_freeze_budget::FreezeResume,
) -> anyhow::Error {
    match judge::JudgeIncomplete::caused(&error) {
        Some(incomplete) if resume.persist => {
            crate::command::workflow_freeze_budget::FreezeIncomplete::stalled(
                incomplete.to_string(),
                &resume.progress,
            )
            .into()
        }
        _ => error,
    }
}

pub(crate) async fn prepare_acceptance_freeze_resumable(
    project_root: &Path,
    tasks_root: &Path,
    prd_path: &Path,
    mode: GateMode,
    candidate: Vec<u8>,
    client: Arc<dyn WorkflowLlmClient>,
    resume: &crate::command::workflow_freeze_budget::FreezeResume,
) -> Result<PreparedAcceptanceFreeze> {
    let freeze_mode = freeze_mode(mode)?;
    let original = candidate;
    let (prd, prd_digest, exact_criteria) = validate_prd_input(prd_path)?;
    let prd_text = String::from_utf8(prd.clone()).context("PRD is not UTF-8")?;
    let expected: BTreeSet<_> = exact_criteria.keys().cloned().collect();
    let mut contract = preflight::prepare(
        project_root,
        prd_path,
        &prd_digest,
        &prd_text,
        &exact_criteria,
        &expected,
        &original,
    )?;

    let judged = resume
        .persist
        .then(|| judge_store::JudgeStore::for_project(project_root, resume.progress.clone()));
    contract = incremental::judge(
        project_root,
        tasks_root,
        client.as_ref(),
        contract,
        &expected,
        judged.as_ref(),
    )
    .await
    .map_err(|error| resumable(error, resume))?;

    // A4: a check that crashes, or passes before any implementation, is
    // never published; it goes back to its author.
    let probed = coverage_gate::pre_implementation_findings(
        project_root,
        tasks_root,
        prd_path,
        &contract,
        resume,
    )
    .await?;
    // Issue 328: the lock records the tree the checks were proven on.
    let baseline_commit = (probed.baseline.as_ref()).map(|runs| runs.commit.clone());
    // Issue 275: nor is one whose failure there is its own setup breaking a
    // rule a correct implementation keeps: it could never pass.
    let probed = passability::judge_baseline_failures(
        project_root,
        tasks_root,
        &prd_text,
        client.as_ref(),
        &mut contract,
        probed,
        resume,
    )
    .await
    .map_err(|error| resumable(error, resume))?;
    findings::finish_acceptance(
        project_root,
        tasks_root,
        prd_path,
        &prd_text,
        freeze_mode,
        &contract,
        probed,
        baseline_commit,
    )
}

pub(crate) fn publish_acceptance_freeze(
    prepared: PreparedAcceptanceFreeze,
    permit: crate::command::workflow_gate::GatePublicationPermit,
) -> Result<FreezeAcceptanceResult> {
    let finding_texts = prepared
        .findings
        .iter()
        .map(|finding| finding.text.clone())
        .collect::<Vec<_>>();
    let publication_identity = prepared.publication_identity();
    if !permit.authorizes(
        GateId::FreezeAcceptance,
        &finding_texts,
        &publication_identity,
    ) {
        return Err(anyhow!(
            "publication permit does not authorize acceptance freeze"
        ));
    }
    prepared.require_all_accepted()?;
    publish_acceptance_files_with_recovery(
        &prepared.tasks_root,
        &prepared.project_root,
        &prepared.contract_bytes,
        &prepared.lock,
        &prepared.pin,
        prepared.recovery.as_ref(),
    )?;
    Ok(prepared.result)
}

#[path = "workflow_task_set_skeleton_freeze.rs"]
mod skeleton_freeze;
pub(crate) use skeleton_freeze::{
    prepare_skeleton_freeze, prepare_skeleton_freeze_from_candidate, publish_skeleton_freeze,
};

fn freeze_mode(mode: GateMode) -> Result<FreezeGateMode> {
    match mode {
        GateMode::Off => Err(anyhow!(
            "gate_mode=off must return before freeze preparation"
        )),
        GateMode::Observe => Ok(FreezeGateMode::Observe),
        GateMode::Enforce => Ok(FreezeGateMode::Enforce),
    }
}

fn project_relative(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/")
}

#[path = "workflow_task_set_check_sources.rs"]
pub(crate) mod check_sources;
#[path = "workflow_acceptance_executability.rs"]
pub(crate) mod executability;
#[path = "workflow_task_set_publish.rs"]
mod publish;
#[path = "workflow_acceptance_reauthor.rs"]
pub(crate) mod reauthor;
#[path = "workflow_acceptance_republish.rs"]
pub(crate) mod republish;
pub(crate) use findings::{
    non_accepted_ids, prepare_acceptance_freeze_reauthoring, prepare_from_judged,
    unproven_incomplete,
};
#[cfg(test)]
use publish::cleanup_committed_backups;
pub(crate) use publish::{
    ChainLock, ChainRead, UnsettledPublish, begin_publish, create_dir_all_durably,
    lock_and_recover, pause_if_unsettled, publish_files_atomically, recover_interrupted_publish,
    register_publish_settle, sync_file, sync_parent, validate_destination,
    validate_existing_parents, write_durably,
};
use publish::{publish_acceptance_files_with_recovery, publish_skeleton_files};

#[cfg(all(test, unix))]
#[path = "workflow_acceptance_passability_tests.rs"]
mod passability_tests;
#[cfg(test)]
#[path = "workflow_acceptance_passability_test_support.rs"]
pub(crate) mod passability_tests_support;
#[cfg(test)]
#[path = "workflow_task_set_tests.rs"]
mod tests;

#[cfg(test)]
pub(crate) use publish::reader_test_step;
#[cfg(test)]
pub(crate) use publish::take_synced_dirs;
#[cfg(test)]
pub(crate) use publish::{crash_publish, stick_next_commit};
#[cfg(test)]
#[path = "workflow_task_set_read_race_tests.rs"]
mod read_race_tests;
