//! Host-owned preparation and publication of a task-skeleton freeze, split
//! out of `workflow_task_set.rs` to hold the 500-line ceiling.

use super::*;

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
        .map(archon_shell::paths::plain)
        .with_context(|| format!("canonicalizing explicit PRD {}", prd_path.display()))?;
    let contracted_prd = {
        let path = PathBuf::from(&contract.prd.path);
        let path = if path.is_absolute() {
            path
        } else {
            project_root.join(path)
        };
        path.canonicalize()
            .map(archon_shell::paths::plain)
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
    let mut defects =
        archon_workflow::task_skeleton::skeleton_defects(&skeleton, &pin.acceptance_digest);
    // Read as the skeleton reader above read it (Issue 312): a field it
    // ignores is never staged, and a strict `Value` read of it would turn a
    // candidate the reader accepted into an operational failure.
    let marker_value = crate::command::workflow::skeleton_document(&candidate)
        .context("candidate marker inspection")?;
    defects.extend(crate::command::workflow_freeze_candidate::marker_defects(
        &marker_value,
        true,
    ));
    let mut findings = findings::skeleton_findings(
        tasks_root,
        &canonical_prd,
        &prd_text,
        &pin,
        &pin_path,
        &skeleton,
    )?;
    // A12: every task needs a frozen check that answers for its work.
    findings.extend(coverage_gate::skeleton_check_findings(
        &skeleton,
        &contract,
        &skeleton_path,
    ));

    if !defects.is_empty() {
        // Shape and set defects are one complete refusal, including siblings
        // that a first-error shape validator used to hide from the author.
        defects.extend(findings.iter().enumerate().map(|(index, finding)| {
            archon_workflow::defect::ValidationDefect {
                identity: finding.deterministic_defect.clone().unwrap_or_else(|| {
                    archon_workflow::defect::DeterministicDefect::new(
                        "skeleton_policy",
                        "skeleton",
                        format!("finding/{index}"),
                    )
                }),
                message: finding.text.clone(),
            }
        }));
        return CandidateRejected::tag(Err(
            crate::command::workflow_task_set_candidate::CandidateDefects(defects).into(),
        ));
    }
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
    // A skeleton freeze is not a per-check republish: the chain starts anew.
    pin.lineage.clear();
    pin.lineage_recording = Some(archon_workflow::task_set_lineage::LINEAGE_RECORDING_V1);
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
