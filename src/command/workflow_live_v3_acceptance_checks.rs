//! Batch G: running the selected checks at the stage's site, as the
//! host's environment allows. Split from `workflow_live_v3_acceptance_exec`
//! for size.
//!
//! Three rules sit here. A tracked project input that diverged from the
//! repository with no recorded landing is put back by the host before the
//! scratch is built (`input_divergence`). A site that cannot be built or run
//! is ONE round-level error, never a per-check result a task is sent to fix
//! (Issue-128). The project-input tripwire over the check commands (they
//! run on the host, outside any agent boundary) is armed around the whole
//! round by `run_acceptance_stage`.

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use archon_workflow::acceptance_scratch::{
    CheckResult, DIRECT_DEFAULT_OUTPUT_BYTES, DirectSite, evaluate_floor_direct, run_check_direct,
};
use archon_workflow::acceptance_world::FrozenCommandRef;
use archon_workflow::task_set_contract::{
    AcceptanceCheck, AcceptanceContract, AcceptanceCriterion,
};
use archon_workflow::{WorkflowResult, WorkflowStore, poll_v2_run_control};

use super::{StageContext, command_reference, git_head, observe_in_scratch};
use crate::command::acceptance_scratch_policy::NativeBinding;

fn direct_site(context: &StageContext) -> DirectSite {
    DirectSite {
        repository: context.repository.clone(),
        project: context.project.clone(),
        // Issue 345: built from the host's by the one check-environment
        // rule (the run's policy, else the default one), never handed it.
        host: archon_workflow::acceptance_check_environment::host_environment(),
        policy: context.binding.as_ref().map(|binding| {
            archon_workflow::acceptance_check_environment::CheckPolicy::configured(&binding.policy)
        }),
        target: None,
        timeout_secs: context.binding.as_ref().map_or(
            archon_workflow::acceptance_scratch::DIRECT_DEFAULT_TIMEOUT_SECS,
            |binding| binding.policy.timeout_secs,
        ),
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
        classification: None,
        acceptance_id: id.into(),
        exit_code: None,
        quota_walk_count: 0,
        stdout: vec![],
        stderr: vec![],
        operational_error: Some(error),
    }
}

/// What one execution of the selected checks produced.
pub(in crate::command) struct Executed {
    /// One result per selected criterion, in order; empty when the site
    /// itself failed.
    pub(in crate::command) results: Vec<CheckResult>,
    /// Batch G: the execution SITE could not be built or run (a scratch
    /// that cannot be assembled, a host command that changed the project's
    /// inputs). A round-level operational error: never a check's verdict,
    /// never a task's to fix.
    pub(in crate::command) site_errors: Vec<String>,
}

/// Execute the selected criteria and return one result per criterion, in
/// order. Command-bearing checks go to the configured site; declarative
/// floors evaluate against the live project root. A run-control stop
/// (pause, cancel) propagates as the error it is — it is an interruption of
/// the round, never a check's verdict.
///
/// Batch G: a site that cannot be built or run is returned in
/// `site_errors`, with no per-check results.
#[allow(clippy::too_many_arguments)]
pub(in crate::command) async fn execute_checks(
    store: &WorkflowStore,
    run_id: &str,
    call_id: &str,
    context: &StageContext,
    contract: &AcceptanceContract,
    chain_digest: &str,
    selected: &[&AcceptanceCriterion],
    evidence_dir: &Path,
) -> WorkflowResult<Executed> {
    let run_root = store.run_dir(run_id);
    let mut host_repairs = Vec::new();
    let mut site_errors = Vec::new();
    // Batch G (C): a tracked input the project's copy of which diverged with
    // no recorded landing is the host's to put back before the scratch is
    // built from it, at the stage's own tip only.
    if let Some(binding) = &context.binding
        && let (Some(head), _) = git_head(&context.repository)
    {
        match restore_diverged(&run_root, binding, head.clone()).await {
            Ok(divergences) => {
                for divergence in &divergences {
                    let line = divergence.describe(&head);
                    tracing::warn!(call_id, "{line}");
                    host_repairs.push(line);
                }
            }
            Err(error) => site_errors.push(format!(
                "the host could not compare the project's tracked inputs with the repository at {head}: {error}"
            )),
        }
    }
    let results = run_selected(
        store,
        run_id,
        call_id,
        context,
        contract,
        chain_digest,
        selected,
        evidence_dir,
        &mut site_errors,
    )
    .await?;
    if !host_repairs.is_empty() {
        let _ = std::fs::create_dir_all(evidence_dir);
        let _ = std::fs::write(
            evidence_dir.join("host-input-repairs.json"),
            serde_json::to_vec_pretty(&host_repairs).unwrap_or_default(),
        );
    }
    Ok(Executed {
        results: if site_errors.is_empty() {
            results
        } else {
            Vec::new()
        },
        site_errors,
    })
}

async fn restore_diverged(
    run_root: &Path,
    binding: &NativeBinding,
    head: String,
) -> Result<Vec<archon_workflow::write_coordinator::input_divergence::InputDivergence>, String> {
    let run_root = run_root.to_path_buf();
    let policy = binding.policy.clone();
    tokio::task::spawn_blocking(move || {
        archon_workflow::write_coordinator::input_divergence::restore_diverged_tracked_inputs(
            &run_root, &policy, &head,
        )
    })
    .await
    .map_err(|e| e.to_string())?
}

#[allow(clippy::too_many_arguments)]
async fn run_selected(
    store: &WorkflowStore,
    run_id: &str,
    call_id: &str,
    context: &StageContext,
    contract: &AcceptanceContract,
    chain_digest: &str,
    selected: &[&AcceptanceCriterion],
    evidence_dir: &Path,
    site_errors: &mut Vec<String>,
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
                // Issue-128: one site failure is the round's, not eleven
                // checks' — fanned out per check it read as eleven failing
                // checks and went to their tasks.
                Err(error) => {
                    site_errors.push(format!(
                        "the acceptance scratch site failed, so no command check ran: {error}"
                    ));
                    return Ok(Vec::new());
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
