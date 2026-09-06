//! Observation transaction: resolve, snapshot, execute, clean up, audit.
use super::process::{CheckResult, run};
use super::*;
use crate::acceptance_world::{FrozenCommandRef, resolve_command};
use crate::task_set_contract::{AcceptanceContract, content_digest};
use std::sync::{Arc, atomic::AtomicBool};

#[derive(Debug, Serialize, Deserialize)]
pub struct ObservationResult {
    pub checks: Vec<CheckResult>,
    pub check_evidence: Vec<CheckEvidence>,
    pub operational_errors: Vec<String>,
    pub command_refs: Vec<FrozenCommandRef>,
    pub command_cwds: Vec<crate::task_set_contract::TrustedCwd>,
    pub policy: ScratchPolicy,
    pub copied_project_manifest: BTreeMap<String, String>,
    pub cleanup_error: Option<String>,
    pub live_roots_unchanged: bool,
    pub teardown_verified: bool,
    pub source_commit: String,
    pub policy_digest: String,
    pub source_manifest_digest: String,
    pub before: BTreeMap<String, BTreeMap<String, String>>,
    pub after: BTreeMap<String, BTreeMap<String, String>>,
}
impl ObservationResult {
    pub fn passed(&self) -> bool {
        self.operational_errors.is_empty()
            && self.live_roots_unchanged
            && self.teardown_verified
            && !self.checks.is_empty()
            && self
                .checks
                .iter()
                .all(|c| c.exit_code == Some(0) && c.operational_error.is_none())
    }
}
fn live(policy: &ScratchPolicy) -> WorkflowResult<BTreeMap<String, BTreeMap<String, String>>> {
    [&policy.repository, &policy.project, &policy.task_root]
        .into_iter()
        .map(|p| Ok((p.display().to_string(), inventory(p)?)))
        .collect()
}
/// The caller supplies an integrity-validated pinned contract and chain digest.
/// Authorization is repeated here before creating scratch or spawning children.
pub async fn observe_commands(
    policy: &ScratchPolicy,
    commit: &str,
    contract: &AcceptanceContract,
    chain_digest: &str,
    refs: &[FrozenCommandRef],
    evidence: &Path,
) -> WorkflowResult<ObservationResult> {
    observe_commands_cancellable(
        policy,
        commit,
        contract,
        chain_digest,
        refs,
        evidence,
        Arc::new(AtomicBool::new(false)),
    )
    .await
}
pub async fn observe_commands_cancellable(
    policy: &ScratchPolicy,
    commit: &str,
    contract: &AcceptanceContract,
    chain_digest: &str,
    refs: &[FrozenCommandRef],
    evidence: &Path,
    cancel: Arc<AtomicBool>,
) -> WorkflowResult<ObservationResult> {
    policy.validate()?;
    let commands = refs
        .iter()
        .map(|r| resolve_command(contract, chain_digest, r))
        .collect::<WorkflowResult<Vec<_>>>()?;
    if commands.is_empty() {
        return Err(invalid(
            "no frozen commands selected for native observation",
        ));
    }
    for root in [&policy.repository, &policy.project, &policy.task_root] {
        if evidence.starts_with(root) {
            return Err(invalid("provisional evidence must be outside live roots"));
        }
    }
    let mut result = ObservationResult {
        checks: vec![],
        check_evidence: vec![],
        operational_errors: vec![],
        command_refs: refs.to_vec(),
        command_cwds: commands.iter().map(|c| c.cwd()).collect(),
        policy: policy.clone(),
        copied_project_manifest: BTreeMap::new(),
        cleanup_error: None,
        live_roots_unchanged: false,
        teardown_verified: false,
        source_commit: commit.into(),
        policy_digest: content_digest(&serde_json::to_vec(policy)?),
        source_manifest_digest: String::new(),
        before: BTreeMap::new(),
        after: BTreeMap::new(),
    };
    let mut roots = None;
    let phase = || control::Control::new(policy.timeout_secs, cancel.clone());
    let execution = async {
        result.before = phase().run(|| live(policy))?;
        roots = Some(phase().run(|| ScratchRoots::prepare_inner(policy, commit))?);
        let roots = roots.as_ref().expect("prepared");
        result.copied_project_manifest = phase().run(|| inventory(roots.project()))?;
        let baseline = phase().run(|| identity::capture(roots, policy))?;
        result.source_manifest_digest = content_digest(&serde_json::to_vec(
            &phase().run(|| roots.source_inventory())?,
        )?);
        for (reference, command) in refs.iter().zip(commands) {
            let before_identity = phase().run(|| identity::capture(roots, policy))?;
            if before_identity != baseline {
                return Err(invalid("native build identity changed before cache reuse"));
            }
            let project_before = phase().run(|| inventory(roots.project()))?;
            let cache_before = phase().run(|| identity::cache_digest(roots))?;
            let attempt =
                execute_check(roots, policy, contract, reference, &command, cancel.clone()).await;
            let mut check =
                attempt.unwrap_or_else(|e| operational(&reference.acceptance_id, e.to_string()));
            let after_identity: WorkflowResult<BuildIdentity> = Ok(baseline.clone());
            match &after_identity {
                Ok(current) if current == &baseline => {}
                _ => check.operational_error = Some(
                    "native scratch source or build identity changed; warm target reuse refused"
                        .into(),
                ),
            }
            let project_after = phase().run(|| inventory(roots.project()));
            let cache_after = phase().run(|| identity::cache_digest(roots));
            let changed_paths = match project_after {
                Ok(after) => identity::changed(&project_before, &after),
                Err(e) => {
                    check.operational_error = Some(e.to_string());
                    vec![]
                }
            };
            if let Err(e) = &cache_after {
                check.operational_error = Some(e.to_string());
            }
            result.check_evidence.push(CheckEvidence {
                acceptance_id: reference.acceptance_id.clone(),
                before_identity,
                after_identity: after_identity.ok(),
                changed_project_paths: changed_paths,
                cargo_cache_before: cache_before,
                cargo_cache_after: cache_after.ok(),
                input_reset: false,
            });
            let stop = check.operational_error.is_some();
            result.checks.push(check);
            if stop {
                break;
            }
        }
        Ok::<_, WorkflowError>(())
    }
    .await;
    if let Err(e) = execution {
        result.operational_errors.push(e.to_string());
    }
    let cleanup = match roots.as_mut() {
        Some(roots) => roots.cleanup(),
        None => Ok(()), // prepare performs its own cleanup before returning an error.
    };
    result.teardown_verified = cleanup.is_ok();
    result.cleanup_error = cleanup.err().map(|e| e.to_string());
    if let Some(e) = &result.cleanup_error {
        result.operational_errors.push(e.clone());
    }
    let audit = control::Control::new(
        policy.timeout_secs,
        std::sync::Arc::new(AtomicBool::new(false)),
    );
    match audit.run(|| live(policy)) {
        Ok(after) => {
            result.after = after;
            result.live_roots_unchanged = !result.before.is_empty()
                && normalized(&result.before) == normalized(&result.after);
        }
        Err(e) => result
            .operational_errors
            .push(format!("after audit failed: {e}")),
    }
    for reference in refs {
        if !result
            .checks
            .iter()
            .any(|c| c.acceptance_id == reference.acceptance_id)
        {
            result.checks.push(operational(
                &reference.acceptance_id,
                result
                    .operational_errors
                    .first()
                    .cloned()
                    .unwrap_or_else(|| "not executed after observation failure".into()),
            ));
        }
    }
    std::fs::create_dir_all(evidence).map_err(|e| WorkflowError::io(evidence, e))?;
    let path = evidence.join("observation.json");
    std::fs::write(&path, serde_json::to_vec_pretty(&result)?)
        .map_err(|e| WorkflowError::io(&path, e))?;
    Ok(result)
}
fn operational(id: &str, error: String) -> CheckResult {
    CheckResult {
        acceptance_id: id.into(),
        exit_code: None,
        stdout: vec![],
        stderr: vec![],
        operational_error: Some(error),
    }
}
fn normalized(
    maps: &BTreeMap<String, BTreeMap<String, String>>,
) -> BTreeMap<String, BTreeMap<String, String>> {
    let mut maps = maps.clone();
    for entries in maps.values_mut() {
        if entries
            .get(".git/worktrees")
            .is_some_and(|v| v == "directory")
            && !entries.keys().any(|p| p.starts_with(".git/worktrees/"))
        {
            entries.remove(".git/worktrees");
        }
    }
    maps
}
async fn execute_check(
    roots: &ScratchRoots,
    policy: &ScratchPolicy,
    contract: &AcceptanceContract,
    reference: &FrozenCommandRef,
    command: &crate::acceptance_world::AuthorizedCommand,
    cancel: Arc<AtomicBool>,
) -> WorkflowResult<CheckResult> {
    if reference.kind == crate::acceptance_world::AcceptanceCommandKind::NestedVerifier {
        let entry = contract
            .acceptance
            .iter()
            .chain(&contract.supplementary)
            .find(|e| e.id == reference.acceptance_id)
            .expect("authorized entry");
        if let crate::task_set_contract::AcceptanceCheck::Floor { contract: floor } = &entry.check {
            let mut prerequisites = floor.clone();
            prerequisites.typed_verifier_command = None;
            let facts = control::Control::new(policy.timeout_secs, cancel.clone())
                .run(|| crate::collect_declarative_floor_facts(roots.project(), &prerequisites))?;
            match crate::evaluate_declarative_floor(&prerequisites, &facts) {
                crate::DeclarativeFloorEvaluation::Passed => {}
                crate::DeclarativeFloorEvaluation::Failed { findings } => {
                    return Ok(CheckResult {
                        acceptance_id: reference.acceptance_id.clone(),
                        exit_code: Some(1),
                        stdout: vec![],
                        stderr: findings.join("; ").into_bytes(),
                        operational_error: None,
                    });
                }
                crate::DeclarativeFloorEvaluation::Deferred { .. } => {
                    let generated =
                        crate::acceptance_world::AuthorizedCommand::floor_prerequisites(
                            roots.project(),
                            floor,
                        )?;
                    let checked = run(
                        roots,
                        policy,
                        &reference.acceptance_id,
                        &generated,
                        cancel.clone(),
                    )
                    .await?;
                    if checked.exit_code != Some(0) || checked.operational_error.is_some() {
                        return Ok(checked);
                    }
                }
            }
        }
    }
    run(roots, policy, &reference.acceptance_id, command, cancel).await
}
