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
    FrozenChainSnapshot, LaunchAcceptance, frozen_chain_snapshot, launch_acceptance_decision,
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

/// What the launch step decided: its lines, and a repair's event detail.
#[derive(Debug, Clone, Default)]
pub(crate) struct LaunchStep {
    pub(crate) lines: Vec<String>,
    pub(crate) repair: Option<serde_json::Value>,
}

impl LaunchStep {
    /// Record the lines in the operator log at `log_path`, and a repair as
    /// an event of run `run_id`.
    pub(crate) fn record(
        &self,
        store: &WorkflowStore,
        run_id: &str,
        log_path: &Path,
    ) -> Result<()> {
        for line in &self.lines {
            crate::command::workflow_decompose_log::append_nofollow_line(log_path, line)?;
        }
        if let Some(detail) = &self.repair {
            let seq = store.next_event_seq(run_id)?;
            WorkflowEventLog::new(store.clone()).emit(
                run_id,
                seq,
                WorkflowEventKind::DecompositionPhaseCompleted,
                detail.clone(),
            )?;
        }
        Ok(())
    }
}

/// The whole launch step for an acceptance stage the launcher found frozen
/// in `frozen`: decide, re-author what this build's rules refuse with a
/// client from `factory`, and read the republished chain into `frozen`.
/// Returns the decision lines and the repair's event detail, if any.
pub(crate) async fn at_launch(
    factory: &dyn archon_workflow::WorkflowLlmClientFactory,
    frozen: &mut FrozenChainSnapshot,
    store: &WorkflowStore,
    (project_root, prd_path, task_root, repository_root): (&Path, &Path, &Path, &Path),
    catalog: &archon_workflow::CommandCapabilityCatalog,
) -> Result<LaunchStep> {
    let mut decision = launch_acceptance_decision(
        frozen.acceptance,
        store,
        (project_root, task_root, repository_root),
        catalog,
    )?;
    if decision.reauthor.is_empty() {
        return Ok(LaunchStep {
            lines: decision.lines,
            repair: None,
        });
    }
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
    let (line, detail) = reauthor_refused(
        client.as_ref(),
        &probe,
        (project_root, prd_path, task_root),
        &decision,
    )
    .await?;
    decision.lines.push(line);
    *frozen = frozen_chain_snapshot(project_root, prd_path, task_root)
        .context("reading the frozen chain the launch republished")?;
    Ok(LaunchStep {
        lines: decision.lines,
        repair: Some(detail),
    })
}
