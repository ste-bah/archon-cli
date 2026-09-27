//! The sanctioned repair of a frozen acceptance contract: re-author named
//! checks, keep every other entry byte-identical, and republish the whole
//! frozen chain — contract, lock, skeleton, skeleton lock and pin — in one
//! atomic transaction, so no follow-up `freeze-skeleton` and no hand edit of
//! the task directory is ever needed.
//!
//! Used by `workflow freeze-acceptance --reauthor` and by the authored run's
//! acceptance stage when a round meets a check the judge did not accept. Also
//! home of the launch guard that refuses to start a run bound to such a check.

use archon_workflow::task_set_contract::{ACCEPTANCE_LOCK_FILE, TASK_SKELETON_LOCK_FILE};
use archon_workflow::task_skeleton::validate_full_chain;

use super::reauthor::AuthorScope;
use super::*;

/// One per-check repair of the contract frozen beside `tasks_root`.
pub(crate) struct ReauthorRequest<'a> {
    pub(crate) project_root: &'a Path,
    pub(crate) tasks_root: &'a Path,
    pub(crate) prd_path: &'a Path,
    pub(crate) mode: GateMode,
    pub(crate) ids: &'a BTreeSet<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ReauthorResult {
    pub(crate) acceptance_digest: String,
    pub(crate) freeze_event_id: String,
    pub(crate) skeleton_digest: Option<String>,
    pub(crate) diagnostics: Vec<String>,
}

/// Everything verified about the chain before any model is asked.
struct Verified {
    pin: AcceptancePin,
    pin_path: PathBuf,
    contract: AcceptanceContract,
    canonical_prd: PathBuf,
    prd_text: String,
    expected: BTreeSet<String>,
    skeleton: Option<TaskSkeleton>,
}

fn verify(request: &ReauthorRequest<'_>) -> Result<Verified> {
    let (project_root, tasks_root) = (request.project_root, request.tasks_root);
    let pin_path = acceptance_pin_path(project_root, tasks_root);
    let pin: AcceptancePin = serde_json::from_slice(&std::fs::read(&pin_path).with_context(
        || {
            format!(
                "acceptance pin {} could not be read; --reauthor repairs a frozen contract, so run the whole-set freeze-acceptance first",
                pin_path.display()
            )
        },
    )?)
    .with_context(|| format!("acceptance pin {} is malformed", pin_path.display()))?;
    let contract_path = tasks_root.join(ACCEPTANCE_CONTRACT_FILE);
    let bytes = std::fs::read(&contract_path)
        .with_context(|| format!("reading {}", contract_path.display()))?;
    let contract: AcceptanceContract = serde_json::from_slice(&bytes)
        .with_context(|| format!("parsing {}", contract_path.display()))?;
    // Byte identity of every entry not named rests on this: the file is the
    // host's own serialization, so re-serializing an entry reproduces it.
    if serde_json::to_vec_pretty(&contract)? != bytes {
        return Err(anyhow!(
            "{} is not in the host's canonical serialization, so the entries not named could not be kept byte-identical; restore the frozen file or re-run the whole-set freeze-acceptance",
            contract_path.display()
        ));
    }
    let canonical_prd = request
        .prd_path
        .canonicalize()
        .with_context(|| format!("canonicalizing PRD {}", request.prd_path.display()))?;
    let contracted = PathBuf::from(&contract.prd.path);
    let contracted = if contracted.is_absolute() {
        contracted
    } else {
        project_root.join(contracted)
    };
    if contracted.canonicalize().ok().as_deref() != Some(canonical_prd.as_path()) {
        return Err(anyhow!(
            "--prd {} is not the contract's frozen PRD {}; pass the frozen PRD",
            canonical_prd.display(),
            contracted.display()
        ));
    }
    let prd = std::fs::read(&canonical_prd)?;
    if content_digest(&prd) != contract.prd.digest {
        return Err(anyhow!(
            "PRD {} changed since the contract was frozen (digest {} != {}); a changed PRD needs the whole-set freeze-acceptance, not a per-check repair",
            canonical_prd.display(),
            content_digest(&prd),
            contract.prd.digest
        ));
    }
    let prd_text = String::from_utf8(prd).context("frozen PRD is not UTF-8")?;
    let expected: BTreeSet<String> = acceptance_criteria(&prd_text).into_keys().collect();
    validate_acceptance_bundle(tasks_root, Some(&pin), &expected)
        .map_err(|error| anyhow!("the frozen acceptance chain does not verify: {error}"))?;
    let known = contract
        .acceptance
        .iter()
        .chain(&contract.supplementary)
        .map(|entry| entry.id.clone())
        .collect::<BTreeSet<_>>();
    if request.ids.is_empty() {
        return Err(anyhow!("--reauthor needs at least one check id"));
    }
    let unknown = request.ids.difference(&known).cloned().collect::<Vec<_>>();
    if !unknown.is_empty() {
        return Err(anyhow!(
            "--reauthor names check(s) not in the frozen contract: {}; the contract holds {}",
            unknown.join(", "),
            known.iter().cloned().collect::<Vec<_>>().join(", ")
        ));
    }
    let unnamed = non_accepted_ids(&contract)
        .difference(request.ids)
        .cloned()
        .collect::<Vec<_>>();
    if !unnamed.is_empty() {
        return Err(anyhow!(
            "the contract also carries check(s) the judge did not accept: {}; name each with --reauthor so the republished contract holds none",
            unnamed.join(", ")
        ));
    }
    let skeleton = match (
        tasks_root.join(TASK_SKELETON_FILE).exists(),
        tasks_root.join(TASK_SKELETON_LOCK_FILE).exists(),
        pin.skeleton_digest.is_some() || pin.skeleton_gate.is_some(),
    ) {
        (false, false, false) => None,
        (true, true, true) => Some(validate_full_chain(tasks_root, &pin).map_err(|error| {
            anyhow!(
                "the frozen skeleton chain does not verify: {error}; run workflow freeze-skeleton to restore it before a per-check repair"
            )
        })?),
        (file, lock, pinned) => {
            return Err(anyhow!(
                "partial skeleton freeze beside {}: file={file}, lock={lock}, pin={pinned}; run workflow freeze-skeleton first",
                tasks_root.display()
            ));
        }
    };
    Ok(Verified {
        pin,
        pin_path,
        contract,
        canonical_prd,
        prd_text,
        expected,
        skeleton,
    })
}

/// Re-author and re-judge exactly `request.ids`, then republish the chain.
/// Nothing is written unless every named check ends accepted and both gates
/// allow publication.
pub(crate) async fn reauthor_and_republish(
    client: &dyn WorkflowLlmClient,
    request: ReauthorRequest<'_>,
    scope: &AuthorScope,
) -> Result<ReauthorResult> {
    let freeze_mode = freeze_mode(request.mode)?;
    let verified = verify(&request)?;
    let tasks_root = request.tasks_root;
    let repaired = reauthor::reauthor(client, &verified.contract, request.ids, scope).await?;
    let still = non_accepted_ids(&repaired);
    if !still.is_empty() {
        return Err(anyhow!(
            "{}",
            reauthor::not_accepted_report(&repaired, &still, reauthor::REAUTHOR_ATTEMPTS)
        ));
    }
    let contract_path = tasks_root.join(ACCEPTANCE_CONTRACT_FILE);
    let contract_bytes = serde_json::to_vec_pretty(&repaired)?;
    let findings = findings::acceptance_findings(
        request.prd_path,
        &verified.prd_text,
        &contract_path,
        &repaired,
    );
    let (lock, mut pin) =
        findings::acceptance_lock_and_pin(tasks_root, freeze_mode, &findings, &contract_bytes);
    // Obligations are unchanged by a per-check repair, so the operator's
    // recorded waivers still describe this task set.
    pin.fidelity_waivers = verified.pin.fidelity_waivers.clone();
    let mut files = vec![
        (contract_path, contract_bytes),
        (
            tasks_root.join(ACCEPTANCE_LOCK_FILE),
            serde_json::to_vec_pretty(&lock)?,
        ),
    ];
    let mut skeleton_findings = Vec::new();
    if let Some(mut skeleton) = verified.skeleton.clone() {
        // The skeleton's content is unchanged; only the acceptance digest it
        // binds moves, so the skeleton gate re-runs over it here instead of in
        // a manual `freeze-skeleton` afterwards.
        skeleton
            .acceptance_digest
            .clone_from(&pin.acceptance_digest);
        validate_skeleton(&skeleton, &pin.acceptance_digest)?;
        skeleton_findings = findings::skeleton_findings(
            tasks_root,
            &verified.canonical_prd,
            &verified.prd_text,
            &pin,
            &verified.pin_path,
            &skeleton,
        )?;
        let stamp = gate_stamp(freeze_mode, &skeleton_findings);
        let skeleton_bytes = serde_json::to_vec_pretty(&skeleton)?;
        let digest = content_digest(&skeleton_bytes);
        let skeleton_lock = TaskSkeletonLock {
            algorithm: "blake3".into(),
            digest: digest.clone(),
            acceptance_digest: pin.acceptance_digest.clone(),
            gate: stamp.clone(),
        };
        pin.skeleton_digest = Some(digest);
        pin.skeleton_gate = Some(stamp);
        files.push((tasks_root.join(TASK_SKELETON_FILE), skeleton_bytes));
        files.push((
            tasks_root.join(TASK_SKELETON_LOCK_FILE),
            serde_json::to_vec_pretty(&skeleton_lock)?,
        ));
    }
    files.push((verified.pin_path.clone(), serde_json::to_vec_pretty(&pin)?));
    let identity = content_digest(&serde_json::to_vec(
        &files
            .iter()
            .map(|(path, bytes)| (path.display().to_string(), content_digest(bytes)))
            .collect::<Vec<_>>(),
    )?);
    let mut diagnostics = Vec::new();
    let mut gates = vec![(GateId::FreezeAcceptance, findings)];
    if verified.skeleton.is_some() {
        gates.push((GateId::FreezeSkeleton, skeleton_findings));
    }
    for (gate, gate_findings) in gates {
        let texts = gate_findings
            .iter()
            .map(|finding| finding.text.clone())
            .collect::<Vec<_>>();
        let mut disposition = crate::command::workflow_gate::run_sync_gate(
            request.project_root,
            request.mode,
            gate,
            || {
                Ok(
                    crate::command::workflow_gate::GateEvaluation::new("", gate_findings)
                        .with_publication_identity(identity.clone()),
                )
            },
        )?;
        diagnostics.extend(disposition.diagnostics().iter().cloned());
        disposition.require_allowed()?;
        let permit = disposition
            .take_publication_permit()
            .ok_or_else(|| anyhow!("per-check repair received no {} permit", gate.as_str()))?;
        if !permit.authorizes(gate, &texts, &identity) {
            return Err(anyhow!(
                "publication permit does not authorize the per-check repair"
            ));
        }
    }
    publish_files_atomically(&files, "workflow freeze-acceptance --reauthor")?;
    // The published chain must verify exactly as every later reader verifies it.
    validate_acceptance_bundle(tasks_root, Some(&pin), &verified.expected)
        .map_err(|error| anyhow!("republished acceptance chain does not verify: {error}"))?;
    if verified.skeleton.is_some() {
        validate_full_chain(tasks_root, &pin)
            .map_err(|error| anyhow!("republished skeleton chain does not verify: {error}"))?;
    }
    Ok(ReauthorResult {
        acceptance_digest: pin.acceptance_digest,
        freeze_event_id: pin.freeze_event_id,
        skeleton_digest: pin.skeleton_digest,
        diagnostics,
    })
}

/// The exact operator command that repairs `ids` in the contract beside
/// `tasks_root`.
pub(crate) fn reauthor_command(
    project_root: &Path,
    tasks_root: &Path,
    contract: &AcceptanceContract,
    ids: &BTreeSet<String>,
) -> String {
    let prd = PathBuf::from(&contract.prd.path);
    let prd = if prd.is_absolute() {
        prd
    } else {
        project_root.join(prd)
    };
    format!(
        "archon workflow freeze-acceptance {} --tasks {} --prd {}",
        ids.iter()
            .map(|id| format!("--reauthor {id}"))
            .collect::<Vec<_>>()
            .join(" "),
        tasks_root.display(),
        prd.display()
    )
}

/// Refuse to launch a run bound to a contract carrying any check the judge
/// did not accept: such a check can never pass, so the run could never
/// complete. Absent or unreadable contracts are the acceptance stage's to
/// report; this guard speaks only to judged verdicts.
pub(crate) fn refuse_unaccepted_launch(project_root: &Path, tasks_root: &Path) -> Result<()> {
    let path = tasks_root.join(ACCEPTANCE_CONTRACT_FILE);
    let Some(contract) = std::fs::read(&path)
        .ok()
        .and_then(|bytes| serde_json::from_slice::<AcceptanceContract>(&bytes).ok())
    else {
        return Ok(());
    };
    let bad = non_accepted_ids(&contract);
    if bad.is_empty() {
        return Ok(());
    }
    Err(anyhow!(
        "refusing to launch: {} is bound to this run and carries {} check(s) the judge did not accept, which can never pass:\n  - {}\nRepair only those entries (the skeleton chain is re-bound in the same step), then launch again:\n  {}",
        path.display(),
        bad.len(),
        reauthor::not_accepted_lines(&contract, &bad).join("\n  - "),
        reauthor_command(project_root, tasks_root, &contract, &bad)
    ))
}

#[cfg(test)]
#[path = "workflow_acceptance_republish_test_fixture.rs"]
pub(crate) mod test_fixture;
#[cfg(test)]
#[path = "workflow_acceptance_republish_tests.rs"]
mod tests;
