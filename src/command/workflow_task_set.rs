//! Host-owned preparation and atomic publication of decomposition freezes.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

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
    validate_acceptance_bundle, validate_acceptance_structure,
};
use archon_workflow::task_set_edges::analyze_task_set_edges;
use archon_workflow::task_skeleton::{
    TaskSkeleton, TaskSkeletonLock, validate_skeleton, validate_skeleton_set,
};

use crate::command::workflow_gate::{GateFinding, GateId};

#[path = "workflow_task_set_judge.rs"]
mod judge;
use judge::{
    apply_judgments, batched_judge_prompt, gate_stamp, judge_contract, predecessor_findings,
    require_complete_judge_response,
};

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

#[path = "workflow_task_set_enforce.rs"]
mod enforce;
#[path = "workflow_task_set_identity.rs"]
mod identity;
#[path = "workflow_task_set_prd.rs"]
mod prd;
#[path = "workflow_task_set_staging.rs"]
mod staging;
pub(crate) use crate::command::workflow_task_set_candidate::CandidateRejected;
pub(crate) use enforce::{freeze_acceptance, freeze_skeleton};
pub(crate) use prd::validate_prd_input;

pub(crate) fn acceptance_pin_path(project_root: &Path, tasks_root: &Path) -> PathBuf {
    let canonical = tasks_root
        .canonicalize()
        .unwrap_or_else(|_| tasks_root.to_path_buf());
    let key = content_digest(canonical.to_string_lossy().as_bytes());
    project_root
        .join(".archon")
        .join("task-set-pins")
        .join(format!("{key}.json"))
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

pub(crate) async fn prepare_acceptance_freeze_from_candidate(
    project_root: &Path,
    tasks_root: &Path,
    prd_path: &Path,
    mode: GateMode,
    candidate: Vec<u8>,
    client: Arc<dyn WorkflowLlmClient>,
) -> Result<PreparedAcceptanceFreeze> {
    let freeze_mode = freeze_mode(mode)?;
    let contract_path = tasks_root.join(ACCEPTANCE_CONTRACT_FILE);
    let original = candidate;
    let (prd, prd_digest, exact_criteria) = validate_prd_input(prd_path)?;
    let prd_text = String::from_utf8(prd.clone()).context("PRD is not UTF-8")?;
    let expected: BTreeSet<_> = exact_criteria.keys().cloned().collect();
    // The candidate is the author's artifact from stdin, not the live contract
    // on disk: naming the live path here sent a reader hunting a file that does
    // not exist yet.
    // Everything up to the judge inspects the author's artifact, so a failure
    // here is the artifact's, not the host's. Tagging it lets the caller feed
    // the reason back to the author instead of ending the run: an id the PRD
    // never defined is exactly the kind of mistake a second attempt fixes.
    let mut contract: AcceptanceContract = CandidateRejected::tag(
        serde_json::from_slice(&original).context("parsing the candidate acceptance contract"),
    )?;
    contract.prd.path = project_relative(project_root, prd_path);
    contract.prd.digest = prd_digest;
    contract.gap_policy.forbidden_phrases = residual_gap_forbidden_phrases(&prd_text);
    contract.gap_policy.required_fields = REQUIRED_RESIDUAL_GAP_FIELDS
        .iter()
        .map(|field| (*field).to_string())
        .collect();
    for criterion in &mut contract.acceptance {
        criterion.criterion =
            CandidateRejected::tag(exact_criteria.get(&criterion.id).cloned().ok_or_else(|| {
                anyhow!(
                    "acceptance id '{}' is not defined by the PRD; remove it or correct the id",
                    criterion.id
                )
            }))?;
    }
    CandidateRejected::tag(
        validate_acceptance_structure(&contract, &expected, false).map_err(anyhow::Error::new),
    )?;

    contract = judge_contract(client.as_ref(), contract, &expected).await?;

    let mut findings = malformed_obligation_ids(&prd_text)
        .into_iter()
        .map(|id| {
            GateFinding::new(
                GateId::FreezeAcceptance,
                malformed_obligation_finding(&id),
                id,
                Some(prd_path.to_path_buf()),
                archon_workflow::RemediationScope::PrdInput,
            )
        })
        .collect::<Vec<_>>();
    findings.extend(duplicate_obligation_ids(&prd_text).into_iter().map(|id| {
        GateFinding::new(
            GateId::FreezeAcceptance,
            duplicate_obligation_finding(&id),
            id,
            Some(prd_path.to_path_buf()),
            archon_workflow::RemediationScope::PrdInput,
        )
    }));
    findings.extend(
        acceptance_policy_findings(&contract)
            .into_iter()
            .map(|finding| {
                let subject = finding
                    .field
                    .split('.')
                    .next()
                    .unwrap_or("acceptance-contract");
                GateFinding::new(
                    GateId::FreezeAcceptance,
                    finding.message,
                    subject,
                    Some(contract_path.clone()),
                    archon_workflow::RemediationScope::CandidateArtifact,
                )
            }),
    );
    let stamp = gate_stamp(freeze_mode, &findings);
    let contract_bytes = serde_json::to_vec_pretty(&contract)?;
    let digest = content_digest(&contract_bytes);
    let lock = AcceptanceLock {
        algorithm: "blake3".into(),
        digest: digest.clone(),
        gate: stamp.clone(),
    };
    let freeze_event_id = format!("acceptance-freeze-{}", &digest[..12]);
    let pin = AcceptancePin {
        task_root: tasks_root
            .canonicalize()
            .unwrap_or_else(|_| tasks_root.to_path_buf())
            .display()
            .to_string(),
        acceptance_digest: digest.clone(),
        freeze_event_id: freeze_event_id.clone(),
        acceptance_gate: stamp,
        skeleton_digest: None,
        skeleton_gate: None,
    };
    Ok(PreparedAcceptanceFreeze {
        tasks_root: tasks_root.to_path_buf(),
        project_root: project_root.to_path_buf(),
        contract_bytes,
        lock,
        pin,
        findings,
        result: FreezeAcceptanceResult {
            acceptance_digest: digest,
            freeze_event_id,
        },
    })
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
    publish_acceptance_files(
        &prepared.tasks_root,
        &prepared.project_root,
        &prepared.contract_bytes,
        &prepared.lock,
        &prepared.pin,
    )?;
    Ok(prepared.result)
}

pub(crate) fn prepare_skeleton_freeze(
    project_root: &Path,
    tasks_root: &Path,
    prd_path: &Path,
    mode: GateMode,
) -> Result<PreparedSkeletonFreeze> {
    let skeleton_path = tasks_root.join(TASK_SKELETON_FILE);
    let candidate = std::fs::read(&skeleton_path)
        .with_context(|| format!("reading task skeleton at {}", skeleton_path.display()))?;
    prepare_skeleton_freeze_from_candidate(project_root, tasks_root, prd_path, mode, candidate)
}

pub(crate) fn prepare_skeleton_freeze_from_candidate(
    project_root: &Path,
    tasks_root: &Path,
    prd_path: &Path,
    mode: GateMode,
    candidate: Vec<u8>,
) -> Result<PreparedSkeletonFreeze> {
    let freeze_mode = freeze_mode(mode)?;
    let pin_path = acceptance_pin_path(project_root, tasks_root);
    let original_pin = std::fs::read(&pin_path).with_context(|| {
        format!(
            "required acceptance pin {} could not be read; run workflow freeze-acceptance first",
            pin_path.display()
        )
    })?;
    let mut pin: AcceptancePin = serde_json::from_slice(&original_pin).with_context(|| {
        format!(
            "acceptance pin {} is malformed or unstamped; re-run workflow freeze-acceptance with the current binary",
            pin_path.display()
        )
    })?;
    let contract_path = tasks_root.join(ACCEPTANCE_CONTRACT_FILE);
    let contract: AcceptanceContract = serde_json::from_slice(&std::fs::read(&contract_path)?)
        .with_context(|| format!("parsing {}", contract_path.display()))?;
    let canonical_prd = prd_path
        .canonicalize()
        .with_context(|| format!("canonicalizing explicit PRD {}", prd_path.display()))?;
    let contracted_prd = {
        let path = PathBuf::from(&contract.prd.path);
        let path = if path.is_absolute() {
            path
        } else {
            project_root.join(path)
        };
        path.canonicalize()
            .with_context(|| format!("canonicalizing contracted PRD {}", path.display()))?
    };
    if canonical_prd != contracted_prd {
        return Err(anyhow!(
            "freeze-skeleton PRD {} does not match acceptance contract PRD {}; pass the exact frozen PRD or re-run workflow freeze-acceptance",
            canonical_prd.display(),
            contracted_prd.display()
        ));
    }
    let prd_bytes = std::fs::read(&canonical_prd)
        .with_context(|| format!("reading frozen PRD at {}", prd_path.display()))?;
    let actual_prd_digest = content_digest(&prd_bytes);
    if actual_prd_digest != contract.prd.digest {
        return Err(anyhow!(
            "PRD digest mismatch for {}: contract expected {}, actual {}; restore the frozen PRD or re-run workflow freeze-acceptance",
            canonical_prd.display(),
            contract.prd.digest,
            actual_prd_digest
        ));
    }
    let prd_text = String::from_utf8(prd_bytes).context("frozen PRD is not UTF-8")?;
    let expected = acceptance_criteria(&prd_text).into_keys().collect();
    validate_acceptance_bundle(tasks_root, Some(&pin), &expected)?;

    let skeleton_path = tasks_root.join(TASK_SKELETON_FILE);
    // The candidate is model-authored, so a wrong shape or a malformed id is
    // the author's mistake: tagged, it returns as a finding the next attempt
    // can act on instead of ending the run as a host malfunction.
    let mut skeleton: TaskSkeleton = CandidateRejected::tag(
        serde_json::from_slice(&candidate)
            .with_context(|| format!("parsing {}", skeleton_path.display())),
    )?;
    skeleton
        .acceptance_digest
        .clone_from(&pin.acceptance_digest);
    CandidateRejected::tag(
        validate_skeleton(&skeleton, &pin.acceptance_digest).map_err(anyhow::Error::from),
    )?;
    let mut findings = malformed_obligation_ids(&prd_text)
        .into_iter()
        .map(|id| {
            GateFinding::new(
                GateId::FreezeSkeleton,
                malformed_obligation_finding(&id),
                id,
                Some(canonical_prd.clone()),
                archon_workflow::RemediationScope::PrdInput,
            )
        })
        .collect::<Vec<_>>();
    findings.extend(duplicate_obligation_ids(&prd_text).into_iter().map(|id| {
        GateFinding::new(
            GateId::FreezeSkeleton,
            duplicate_obligation_finding(&id),
            id,
            Some(canonical_prd.clone()),
            archon_workflow::RemediationScope::PrdInput,
        )
    }));
    predecessor_findings(&pin, &pin_path, &mut findings);
    let expected_obligations = archon_workflow::obligation_ids::obligation_ids(&prd_text);
    findings.extend(
        validate_skeleton_set(&skeleton, &expected_obligations)
            .into_iter()
            .map(|finding| {
                GateFinding::new(
                    GateId::FreezeSkeleton,
                    format!("{}: {}", finding.field, finding.message),
                    finding.field,
                    Some(skeleton_path.clone()),
                    archon_workflow::RemediationScope::Skeleton,
                )
            }),
    );
    let edge_analysis = analyze_task_set_edges(&skeleton);
    findings.extend(edge_analysis.blockers.into_iter().map(|finding| {
        GateFinding::new(
            GateId::FreezeSkeleton,
            format!("{}: {}", finding.field, finding.message),
            finding.field,
            Some(skeleton_path.clone()),
            archon_workflow::RemediationScope::Skeleton,
        )
    }));

    let stamp = gate_stamp(freeze_mode, &findings);
    let skeleton_bytes = serde_json::to_vec_pretty(&skeleton)?;
    let digest = content_digest(&skeleton_bytes);
    let lock = TaskSkeletonLock {
        algorithm: "blake3".into(),
        digest: digest.clone(),
        acceptance_digest: pin.acceptance_digest.clone(),
        gate: stamp.clone(),
    };
    pin.skeleton_digest = Some(digest.clone());
    pin.skeleton_gate = Some(stamp);
    let result = FreezeSkeletonResult {
        skeleton_digest: digest,
        acceptance_digest: pin.acceptance_digest.clone(),
    };
    Ok(PreparedSkeletonFreeze {
        tasks_root: tasks_root.to_path_buf(),
        pin_path,
        skeleton_bytes,
        lock,
        pin,
        findings,
        result,
    })
}

pub(crate) fn publish_skeleton_freeze(
    prepared: PreparedSkeletonFreeze,
    permit: crate::command::workflow_gate::GatePublicationPermit,
) -> Result<FreezeSkeletonResult> {
    let finding_texts = prepared
        .findings
        .iter()
        .map(|finding| finding.text.clone())
        .collect::<Vec<_>>();
    let publication_identity = prepared.publication_identity();
    if !permit.authorizes(
        GateId::FreezeSkeleton,
        &finding_texts,
        &publication_identity,
    ) {
        return Err(anyhow!(
            "publication permit does not authorize skeleton freeze"
        ));
    }
    publish_skeleton_files(
        &prepared.tasks_root,
        &prepared.pin_path,
        &prepared.skeleton_bytes,
        &prepared.lock,
        &prepared.pin,
    )?;
    Ok(prepared.result)
}

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

#[path = "workflow_task_set_publish.rs"]
mod publish;
#[cfg(test)]
use publish::cleanup_committed_backups;
pub(crate) use publish::publish_files_atomically;
use publish::{publish_acceptance_files, publish_skeleton_files};

#[cfg(test)]
#[path = "workflow_task_set_tests.rs"]
mod tests;
