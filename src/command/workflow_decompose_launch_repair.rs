//! Re-authoring the frozen checks a launch's rules refuse (Issue 387, item 3).
//! Kept outside `workflow_decompose_frozen_chain`, so the verify-frozen
//! capabilities' logic closure does not take in the re-author.
//!
//! The workflow repairs, it does not refuse: a frozen check this build's
//! acceptance entry rules refuse goes through the sanctioned per-check
//! repair (`reauthor_and_republish`, the path `freeze-acceptance --reauthor`
//! takes) with each refusal as its author's first finding. Only those checks
//! are re-authored and re-judged, by the freeze-time judge; every other entry
//! stays byte-identical, and the chain (contract, lock, skeleton, skeleton
//! lock, pin) is republished in one transaction. The launch then reads the
//! frozen chain again and verifies it in place like any other frozen stage.

use std::collections::BTreeSet;
use std::path::Path;

use anyhow::{Context, Result, anyhow};
use archon_workflow::{WorkflowEventKind, WorkflowEventLog, WorkflowLlmClient, WorkflowStore};

use crate::command::workflow_decompose_frozen_chain::{
    FrozenChainSnapshot, LaunchAcceptance, frozen_chain_snapshot,
};
use crate::command::workflow_task_set::executability::ExecutabilityProbe;
use crate::command::workflow_task_set::reauthor::{AuthorScope, ReauthorGate};
use crate::command::workflow_task_set::republish::{ReauthorRequest, reauthor_and_republish};

/// The event detail that records a launch repair once the run exists.
pub(crate) const REAUTHORED_EVENT: &str = "frozen_acceptance_checks_reauthored";

/// Re-author every check `decision` routed, with `client` as author and
/// judge and `probe` as the executability gate. Returns the line naming the
/// republished chain, and the event detail recording the repair. A repair
/// that cannot finish is an error naming the checks it was routed for, the
/// rule that refused each one, and why it stopped.
pub(crate) async fn reauthor_refused(
    client: &dyn WorkflowLlmClient,
    probe: &dyn ExecutabilityProbe,
    (project_root, prd_path, task_root): (&Path, &Path, &Path),
    decision: &LaunchAcceptance,
) -> Result<(String, serde_json::Value)> {
    let ids: BTreeSet<String> = decision.reauthor.keys().cloned().collect();
    let named = ids.iter().cloned().collect::<Vec<_>>().join(",");
    let scope = AuthorScope::for_task_set(project_root, task_root, prd_path)
        .map_err(|why| anyhow!("re-authoring frozen check(s) {named}: {why}"))?;
    let trigger = format!(
        "workflow decompose launch: this build's acceptance entry rules refused check(s) {named} ({})",
        decision.recorded
    );
    let result = reauthor_and_republish(
        client,
        ReauthorRequest {
            project_root,
            tasks_root: task_root,
            prd_path,
            ids: &ids,
            gate: ReauthorGate {
                probe,
                seeds: &decision.reauthor,
            },
            trigger: &trigger,
        },
        &scope,
    )
    .await
    .with_context(|| {
        format!(
            "the launch routed frozen check(s) {named} to re-authoring because this build's acceptance entry rules refuse them ({}), and the repair did not finish",
            decision.reauthor.values().cloned().collect::<Vec<_>>().join(" | ")
        )
    })?;
    for diagnostic in &result.diagnostics {
        eprintln!("{diagnostic}");
    }
    let line = format!(
        "frozen_chain_launch acceptance=reauthored ids={named} acceptance_digest={} freeze_event={} skeleton_digest={}",
        result.acceptance_digest,
        result.freeze_event_id,
        result.skeleton_digest.as_deref().unwrap_or("none")
    );
    let detail = serde_json::json!({
        "event": REAUTHORED_EVENT,
        "ids": ids,
        "refusals": decision.reauthor,
        "recorded_logic": decision.recorded,
        "acceptance_digest": result.acceptance_digest,
        "freeze_event_id": result.freeze_event_id,
        "skeleton_digest": result.skeleton_digest,
    });
    Ok((line, detail))
}

/// The launch's acceptance step, run only once the launch holds the claim
/// on its task root (`create_claimed_run`): a chain another run owns is never
/// re-authored. Writes `decision`'s lines to the operator log at `log_path`
/// first, so a repair that then fails is still on record; re-authors what
/// this build's rules refuse with a client from `factory`; records the
/// repair as an event of run `run_id`; and requires the republished chain to
/// be the one the run was launched with (`launched`: the same stages,
/// subjects and bodies). Every line is also shown through `ui`.
pub(crate) async fn after_claim(
    factory: &dyn archon_workflow::WorkflowLlmClientFactory,
    decision: LaunchAcceptance,
    launched: &FrozenChainSnapshot,
    (store, run_id, log_path, ui): (
        &WorkflowStore,
        &str,
        &Path,
        &archon_workflow::SharedWorkflowUiSink,
    ),
    (project_root, prd_path, task_root, repository_root): (&Path, &Path, &Path, &Path),
) -> Result<()> {
    let log =
        |line: &str| crate::command::workflow_decompose_log::append_nofollow_line(log_path, line);
    let mut lines = decision.lines.clone();
    for line in &lines {
        log(line)?;
    }
    if !decision.reauthor.is_empty() {
        let repaired = async {
            let client = factory
                .build_client(archon_workflow::WorkflowLlmClientRequest {
                    cwd: project_root.to_path_buf(),
                    origin: "workflow-decompose-launch-reauthor".into(),
                    session_id: format!(
                        "workflow-decompose-launch-reauthor-{}",
                        uuid::Uuid::new_v4()
                    ),
                    read_roots: vec![repository_root.to_path_buf()],
                })
                .await
                .context("building the client that re-authors refused frozen checks")?;
            let probe = crate::command::workflow_task_set::executability::HostProbe::for_task_set(
                project_root,
                task_root,
            );
            reauthor_refused(
                client.as_ref(),
                &probe,
                (project_root, prd_path, task_root),
                &decision,
            )
            .await
        }
        .await;
        let (line, detail) = match repaired {
            Ok(repaired) => repaired,
            Err(error) => {
                log(&format!(
                    "frozen_chain_launch acceptance=reauthor_failed ids={} error={}",
                    decision
                        .reauthor
                        .keys()
                        .cloned()
                        .collect::<Vec<_>>()
                        .join(","),
                    crate::command::workflow_decompose_events::log_field(&format!("{error:#}"))
                ))?;
                return Err(error);
            }
        };
        log(&line)?;
        let seq = store.next_event_seq(run_id)?;
        WorkflowEventLog::new(store.clone()).emit(
            run_id,
            seq,
            WorkflowEventKind::DecompositionPhaseCompleted,
            detail,
        )?;
        lines.push(line);
        let republished = frozen_chain_snapshot(project_root, prd_path, task_root)
            .context("reading the frozen chain the launch republished")?;
        if republished.to_argument() != launched.to_argument() {
            return Err(anyhow!(
                "the per-check repair republished a frozen chain under {} that differs from the one this run was launched with ({} != {})",
                task_root.display(),
                republished.to_argument(),
                launched.to_argument()
            ));
        }
    }
    for line in &lines {
        ui.emit(archon_workflow::WorkflowUiEvent::Text(format!("{line}\n")))
            .await
            .map_err(|error| anyhow!("reporting {line}: {error}"))?;
    }
    Ok(())
}
