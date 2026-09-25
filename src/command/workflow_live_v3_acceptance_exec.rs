//! Where and how the authored run's acceptance stage executes its checks.
//!
//! Two sites, one runner. With `[workflow.acceptance_execution]` configured,
//! command-bearing checks run through the guardian's hermetic scratch
//! observation at the target repository's current HEAD — the R2 machinery,
//! over the checks the stage selected (every round: the whole contract).
//! Without it, they run directly in the
//! run's target repository checkout under the host environment agents get,
//! and the record says so. Declarative floors evaluate against the live
//! project root either way, as the R2 observer evaluated them.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use archon_workflow::acceptance_scratch::{
    CheckResult, DIRECT_DEFAULT_OUTPUT_BYTES, DIRECT_DEFAULT_TIMEOUT_SECS, DirectSite,
    evaluate_floor_direct, run_check_direct,
};
use archon_workflow::acceptance_world::{AcceptanceCommandKind, FrozenCommandRef};
use archon_workflow::task_set_contract::{
    ACCEPTANCE_CONTRACT_FILE, ACCEPTANCE_LOCK_FILE, AcceptanceCheck, AcceptanceContract,
    AcceptanceCriterion, AcceptancePin, content_digest, validate_acceptance_bundle,
};
use archon_workflow::task_universe::WorkflowV2TaskUniverse;
use archon_workflow::v2::acceptance_stage::AcceptanceExecutionRecordV1;
use archon_workflow::{WorkflowError, WorkflowResult, WorkflowStore, poll_v2_run_control};

use crate::command::acceptance_scratch_policy::NativeBinding;

/// Everything the stage resolved about the run before touching a check.
pub(super) struct StageContext {
    pub(super) project: PathBuf,
    pub(super) task_root: PathBuf,
    pub(super) repository: PathBuf,
    pub(super) binding: Option<NativeBinding>,
}

impl StageContext {
    pub(super) fn contract_path(&self) -> PathBuf {
        self.task_root.join(ACCEPTANCE_CONTRACT_FILE)
    }

    pub(super) fn execution_record(&self) -> AcceptanceExecutionRecordV1 {
        let (mode, environment, timeout_secs) = match &self.binding {
            Some(binding) => (
                "scratch",
                format!(
                    "[workflow.acceptance_execution]: toolchain {} with allowlisted host keys {:?}",
                    binding.policy.toolchain_path, binding.policy.environment_allowlist
                ),
                binding.policy.timeout_secs,
            ),
            None => (
                "direct",
                "no [workflow.acceptance_execution] configured: checks ran in the run's target repository root under the host environment agent shells receive (engine credentials withheld)".to_string(),
                DIRECT_DEFAULT_TIMEOUT_SECS,
            ),
        };
        let (source_commit, dirty_worktree) = git_head(&self.repository);
        AcceptanceExecutionRecordV1 {
            mode: mode.to_string(),
            repository: self.repository.display().to_string(),
            project: self.project.display().to_string(),
            task_root: self.task_root.display().to_string(),
            source_commit,
            dirty_worktree,
            environment,
            timeout_secs,
            config_present: self.binding.is_some(),
        }
    }
}

/// Resolve roots and policy. Errors here are the stage's operational errors:
/// the round is recorded as unevaluable, never as passed.
pub(super) fn resolve_context(
    store: &WorkflowStore,
    run_id: &str,
    target_repository_root: Option<&str>,
    universe: Option<&WorkflowV2TaskUniverse>,
) -> WorkflowResult<StageContext> {
    let project = super::super::workflow_run_end_snapshot::project_root(store)
        .ok_or_else(|| {
            WorkflowError::StateCorrupt("workflow store has no project root".to_string())
        })?
        .to_path_buf();
    let snapshot = super::super::load_generated_v2_metadata(store, run_id)?
        .and_then(|metadata| metadata.observer_snapshot);
    let task_root = snapshot
        .as_ref()
        .map(|snapshot| PathBuf::from(&snapshot.canonical_task_root_identity))
        .filter(|root| root.is_dir())
        .or_else(|| {
            universe.and_then(|universe| {
                super::super::workflow_run_end_snapshot::canonical_task_root(&project, universe)
            })
        })
        .ok_or_else(|| {
            WorkflowError::SpecInvalid(
                "acceptance stage cannot resolve the task set root: the task universe names no single task directory".to_string(),
            )
        })?;
    let binding = match snapshot
        .as_ref()
        .and_then(|snapshot| snapshot.native_execution.clone())
        .filter(|value| value.get("policy").is_some())
        .map(serde_json::from_value::<NativeBinding>)
        .transpose()?
    {
        Some(binding) => Some(binding),
        None => crate::command::acceptance_scratch_policy::capture(&project, &task_root)?,
    };
    let repository = match target_repository_root
        .map(str::trim)
        .filter(|root| !root.is_empty())
    {
        Some(root) => Path::new(root)
            .canonicalize()
            .map_err(|source| WorkflowError::Io {
                path: PathBuf::from(root),
                source,
            })?,
        None => binding
            .as_ref()
            .map(|binding| binding.policy.repository.clone())
            .unwrap_or_else(|| project.clone()),
    };
    if let Some(binding) = &binding
        && binding.policy.repository.canonicalize().ok().as_deref() != Some(repository.as_path())
    {
        return Err(WorkflowError::SpecInvalid(format!(
            "[workflow.acceptance_execution].repository ({}) is not the run's target repository ({}); acceptance must check the repository the run implemented",
            binding.policy.repository.display(),
            repository.display()
        )));
    }
    Ok(StageContext {
        project,
        task_root,
        repository,
        binding,
    })
}

/// Read the contract and, when a lock exists, hold it to the frozen chain.
/// Returns the contract with its chain digest and whether it was verified
/// frozen.
pub(super) fn load_contract(
    context: &StageContext,
) -> WorkflowResult<(AcceptanceContract, String, bool)> {
    let path = context.contract_path();
    let raw = std::fs::read(&path).map_err(|source| WorkflowError::Io {
        path: path.clone(),
        source,
    })?;
    let contract: AcceptanceContract = serde_json::from_slice(&raw)?;
    let digest = content_digest(&raw);
    if !context.task_root.join(ACCEPTANCE_LOCK_FILE).exists() {
        return Ok((contract, digest, false));
    }
    let pin = read_pin(context)?;
    let ids = contract
        .acceptance
        .iter()
        .map(|criterion| criterion.id.clone())
        .collect();
    validate_acceptance_bundle(&context.task_root, pin.as_ref(), &ids)
        .map_err(|error| WorkflowError::ArtifactInvalid(error.to_string()))?;
    Ok((contract, digest, true))
}

/// Why the task set is known to have an acceptance contract, if it is: a
/// freeze lock or pin for it, or a task naming checks it implements.
pub(super) fn contract_declaration(
    context: &StageContext,
    universe: Option<&WorkflowV2TaskUniverse>,
) -> Option<String> {
    if context.task_root.join(ACCEPTANCE_LOCK_FILE).exists() {
        return Some(format!("{ACCEPTANCE_LOCK_FILE} is present"));
    }
    if pin_path(context).exists() {
        return Some(format!("a pin exists at {}", pin_path(context).display()));
    }
    universe
        .into_iter()
        .flat_map(|universe| &universe.tasks)
        .find(|task| !task.implements.is_empty())
        .map(|task| {
            format!(
                "task {} implements {}",
                task.canonical_task_id,
                task.implements.join(", ")
            )
        })
}

fn pin_path(context: &StageContext) -> PathBuf {
    crate::command::workflow_task_set::acceptance_pin_path(&context.project, &context.task_root)
}

fn read_pin(context: &StageContext) -> WorkflowResult<Option<AcceptancePin>> {
    let path = pin_path(context);
    if !path.exists() {
        return Ok(None);
    }
    let bytes = std::fs::read(&path).map_err(|source| WorkflowError::Io {
        path: path.clone(),
        source,
    })?;
    Ok(Some(serde_json::from_slice(&bytes)?))
}

pub(super) fn check_kind(criterion: &AcceptanceCriterion) -> &'static str {
    match &criterion.check {
        AcceptanceCheck::Command { .. } => "command",
        AcceptanceCheck::Floor { contract }
            if contract
                .typed_verifier_command
                .as_deref()
                .is_some_and(|command| !command.trim().is_empty()) =>
        {
            "floor_verifier"
        }
        AcceptanceCheck::Floor { .. } => "floor",
    }
}

fn command_reference(
    criterion: &AcceptanceCriterion,
    chain_digest: &str,
) -> Option<FrozenCommandRef> {
    let (kind, command) = match &criterion.check {
        AcceptanceCheck::Command { command, .. } => {
            (AcceptanceCommandKind::Command, command.as_str())
        }
        AcceptanceCheck::Floor { contract } => (
            AcceptanceCommandKind::NestedVerifier,
            // A blank verifier is a declarative floor, not an empty command
            // that would exit 0 and read as a pass.
            contract
                .typed_verifier_command
                .as_deref()
                .filter(|command| !command.trim().is_empty())?,
        ),
    };
    Some(FrozenCommandRef {
        acceptance_id: criterion.id.clone(),
        kind,
        chain_digest: chain_digest.to_string(),
        command_digest: content_digest(command.as_bytes()),
    })
}

fn direct_site(context: &StageContext) -> DirectSite {
    DirectSite {
        repository: context.repository.clone(),
        project: context.project.clone(),
        environment: archon_tools::bash::host_env()
            .into_iter()
            .collect::<BTreeMap<_, _>>(),
        timeout_secs: context
            .binding
            .as_ref()
            .map_or(DIRECT_DEFAULT_TIMEOUT_SECS, |binding| {
                binding.policy.timeout_secs
            }),
        output_bytes: context
            .binding
            .as_ref()
            .map_or(DIRECT_DEFAULT_OUTPUT_BYTES, |binding| {
                binding.policy.output_bytes
            }),
    }
}

fn operational(id: &str, error: String) -> CheckResult {
    CheckResult {
        acceptance_id: id.into(),
        exit_code: None,
        quota_walk_count: 0,
        stdout: vec![],
        stderr: vec![],
        operational_error: Some(error),
    }
}

/// Execute the selected criteria and return one result per criterion, in
/// order. Command-bearing checks go to the configured site; declarative
/// floors evaluate against the live project root. A run-control stop
/// (pause, cancel) propagates as the error it is — it is an interruption of
/// the round, never a check's verdict.
#[allow(clippy::too_many_arguments)]
pub(super) async fn execute_checks(
    store: &WorkflowStore,
    run_id: &str,
    call_id: &str,
    context: &StageContext,
    contract: &AcceptanceContract,
    chain_digest: &str,
    selected: &[&AcceptanceCriterion],
    evidence_dir: &Path,
) -> WorkflowResult<Vec<CheckResult>> {
    let cancel = Arc::new(AtomicBool::new(false));
    let site = direct_site(context);
    let mut results: BTreeMap<String, CheckResult> = BTreeMap::new();
    let command_refs: Vec<FrozenCommandRef> = selected
        .iter()
        .filter_map(|criterion| command_reference(criterion, chain_digest))
        .collect();
    match &context.binding {
        Some(binding) if !command_refs.is_empty() => {
            let observed = observe_in_scratch(context, binding, &command_refs, evidence_dir).await;
            match observed {
                Ok(checks) => {
                    for check in checks {
                        results.insert(check.acceptance_id.clone(), check);
                    }
                    for reference in &command_refs {
                        results
                            .entry(reference.acceptance_id.clone())
                            .or_insert_with(|| {
                                operational(
                                    &reference.acceptance_id,
                                    "scratch observation returned no result for this check".into(),
                                )
                            });
                    }
                }
                Err(error) => {
                    for reference in &command_refs {
                        results.insert(
                            reference.acceptance_id.clone(),
                            operational(&reference.acceptance_id, error.to_string()),
                        );
                    }
                }
            }
        }
        _ => {
            for reference in &command_refs {
                poll_v2_run_control(store, run_id, call_id)?;
                let result =
                    run_check_direct(&site, contract, chain_digest, reference, cancel.clone())
                        .await
                        .unwrap_or_else(|error| {
                            operational(&reference.acceptance_id, error.to_string())
                        });
                results.insert(reference.acceptance_id.clone(), result);
            }
        }
    }
    for criterion in selected {
        if results.contains_key(&criterion.id) {
            continue;
        }
        let AcceptanceCheck::Floor { contract: floor } = &criterion.check else {
            continue;
        };
        poll_v2_run_control(store, run_id, call_id)?;
        let result = evaluate_floor_direct(&site, &criterion.id, floor, cancel.clone())
            .await
            .unwrap_or_else(|error| operational(&criterion.id, error.to_string()));
        results.insert(criterion.id.clone(), result);
    }
    Ok(selected
        .iter()
        .map(|criterion| {
            results.remove(&criterion.id).unwrap_or_else(|| {
                operational(&criterion.id, "check was not evaluated".to_string())
            })
        })
        .collect())
}

/// The guardian's hermetic observation at the repository's current HEAD,
/// narrowed to the requested checks. Evidence lives under the policy's
/// scratch parent (it may not sit inside a live root); the observation
/// summary is copied beside the round record afterwards.
async fn observe_in_scratch(
    context: &StageContext,
    binding: &NativeBinding,
    refs: &[FrozenCommandRef],
    evidence_dir: &Path,
) -> WorkflowResult<Vec<CheckResult>> {
    let (head, _) = git_head(&context.repository);
    let source_commit = head.ok_or_else(|| {
        WorkflowError::StateCorrupt(format!(
            "cannot read HEAD of {} for the acceptance stage",
            context.repository.display()
        ))
    })?;
    let pin_path = pin_path(context);
    let pin_bytes = std::fs::read(&pin_path).map_err(|_| {
        WorkflowError::SpecInvalid(format!(
            "[workflow.acceptance_execution] observation requires the acceptance pin at {}; freeze the task set or remove the policy to run checks directly",
            pin_path.display()
        ))
    })?;
    let evidence = binding
        .policy
        .scratch_parent
        .join(format!("acceptance-evidence-{}", uuid::Uuid::new_v4()));
    let selection: BTreeSet<String> = refs.iter().map(|r| r.acceptance_id.clone()).collect();
    let request = crate::command::acceptance_scratch_guardian::Request {
        policy: binding.policy.clone(),
        source_commit,
        pin_path,
        expected_pin_digest: content_digest(&pin_bytes),
        evidence: evidence.clone(),
    };
    let result =
        crate::command::acceptance_scratch_guardian::launch_selected(request, Some(selection))
            .await;
    if let Ok(bytes) = std::fs::read(evidence.join("observation.json")) {
        let _ = std::fs::create_dir_all(evidence_dir);
        let _ = std::fs::write(evidence_dir.join("scratch-observation.json"), bytes);
    }
    let result = result?;
    if !result.operational_errors.is_empty() {
        return Err(WorkflowError::StageFailed(format!(
            "scratch observation failed: {}",
            result.operational_errors.join("; ")
        )));
    }
    Ok(result.checks)
}

/// The repository's HEAD as a full object id, and whether the worktree is
/// dirty. `None` when the root is not a git checkout.
pub(super) fn git_head(repository: &Path) -> (Option<String>, bool) {
    let git = |args: &[&str]| {
        std::process::Command::new("git")
            .arg("-C")
            .arg(repository)
            .args(args)
            .output()
            .ok()
            .filter(|output| output.status.success())
            .map(|output| String::from_utf8_lossy(&output.stdout).trim().to_string())
    };
    let head = git(&["rev-parse", "HEAD"])
        .filter(|head| head.len() == 40 && head.bytes().all(|b| b.is_ascii_hexdigit()));
    let dirty = git(&["status", "--porcelain"]).is_some_and(|status| !status.is_empty());
    (head, dirty)
}
