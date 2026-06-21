use std::path::Path;

use anyhow::Result;
use archon_tui::app::TuiEvent;
use archon_tui::event_channel::TuiEventSender;
use archon_workflow::{
    LifecycleAction, LifecycleController, WorkflowApprovalDecision, WorkflowApprovalInspection,
    WorkflowApprovalRecord, WorkflowApprovalStore, WorkflowBundle, WorkflowRun, WorkflowStore,
};

use super::LiveApprovalMode;

pub(super) enum LiveApprovalOutcome {
    Proceed { run: WorkflowRun, note: String },
    Pending(String),
    Denied(String),
}

pub(super) fn gate_live_approval(
    cwd: &Path,
    store: &WorkflowStore,
    run: WorkflowRun,
    approval_mode: LiveApprovalMode,
    tui_tx: &TuiEventSender,
) -> Result<LiveApprovalOutcome> {
    let run_dir = store.run_dir(&run.id);
    if !run_dir.join(archon_workflow::bundle::HARNESS_FILE).exists()
        || !run_dir
            .join(archon_workflow::bundle::COMPILED_SPEC_FILE)
            .exists()
    {
        WorkflowBundle::synthesize_for_imported_spec(store, &run)?;
    }
    let approvals = WorkflowApprovalStore::project(cwd);
    if approval_mode == LiveApprovalMode::CliYes {
        let record = approvals.approve_run_once(cwd, store, &run, approval_mode.decided_by())?;
        let note = render_approval_note(&record, approvals.path());
        let _ = tui_tx.send(TuiEvent::TextDelta(note.clone()));
        return Ok(LiveApprovalOutcome::Proceed { run, note });
    }

    let inspection = approvals.inspect_run(cwd, store, &run)?;
    match inspection.decision.as_ref().map(|record| &record.decision) {
        Some(WorkflowApprovalDecision::AlwaysForProject) => {
            let record = inspection.decision.as_ref().expect("decision exists");
            let note = render_approval_note(record, approvals.path());
            let _ = tui_tx.send(TuiEvent::TextDelta(note.clone()));
            Ok(LiveApprovalOutcome::Proceed { run, note })
        }
        Some(WorkflowApprovalDecision::RunOnce)
            if inspection
                .decision
                .as_ref()
                .and_then(|record| record.run_id.as_deref())
                == Some(run.id.as_str()) =>
        {
            let record = inspection.decision.as_ref().expect("decision exists");
            let note = render_approval_note(record, approvals.path());
            let _ = tui_tx.send(TuiEvent::TextDelta(note.clone()));
            Ok(LiveApprovalOutcome::Proceed { run, note })
        }
        Some(WorkflowApprovalDecision::Denied) => {
            let cancelled =
                LifecycleController::new(store.clone()).apply(&run.id, LifecycleAction::Cancel)?;
            let message = render_denied_note(&inspection, approvals.path(), &cancelled.id);
            let _ = tui_tx.send(TuiEvent::TextDelta(message.clone()));
            Ok(LiveApprovalOutcome::Denied(message))
        }
        _ => {
            let paused =
                LifecycleController::new(store.clone()).apply(&run.id, LifecycleAction::Pause)?;
            let message = render_approval_request(&inspection, approvals.path(), &paused.id);
            let _ = tui_tx.send(TuiEvent::TextDelta(message.clone()));
            Ok(LiveApprovalOutcome::Pending(message))
        }
    }
}

fn render_approval_note(record: &WorkflowApprovalRecord, approvals_path: &Path) -> String {
    let decision = match &record.decision {
        WorkflowApprovalDecision::RunOnce => "run once",
        WorkflowApprovalDecision::AlwaysForProject => "always for this project",
        WorkflowApprovalDecision::Denied => "denied",
    };
    let write_stages = if record.write_capable_stages.is_empty() {
        "none".to_string()
    } else {
        record.write_capable_stages.join(", ")
    };
    let external = if record.external_requirements.is_empty() {
        "none".to_string()
    } else {
        record.external_requirements.join(", ")
    };
    format!(
        "Workflow approval recorded: {decision}\n\
         Workflow: {}\n\
         Phases: {} | max agents: {} | max parallelism: {}\n\
         Write-capable stages: {write_stages}\n\
         External requirements: {external}\n\
         Raw script: {}\n\
         Approval store: {}\n\
         Cost/rate-limit: live provider token and rate limits apply.\n",
        record.workflow_name,
        record.phase_count,
        record.max_agents,
        record.max_parallelism,
        record.raw_script_path,
        approvals_path.display()
    )
}

fn render_approval_request(
    inspection: &WorkflowApprovalInspection,
    approvals_path: &Path,
    run_id: &str,
) -> String {
    let write_stages = if inspection.write_capable_stages.is_empty() {
        "none".to_string()
    } else {
        inspection.write_capable_stages.join(", ")
    };
    let external = if inspection.external_requirements.is_empty() {
        "none".to_string()
    } else {
        inspection.external_requirements.join(", ")
    };
    let raw_script_preview = raw_script_preview(&inspection.raw_script_path);
    format!(
        "Workflow awaiting approval: {run_id}\n\
         Workflow: {}\n\
         Phases: {} | max agents: {} | max parallelism: {}\n\
         Write-capable stages: {write_stages}\n\
         External requirements: {external}\n\
         Raw script: {}\n\
         Compiled spec: {}\n\
         Raw script preview:\n{}\n\
         Edit before approval: {}\n\
         Approval store: {}\n\
         Cost/rate-limit: {}\n\
         Approve once: /workflow approve-run-once {run_id}\n\
         Approve always: /workflow approve-always {run_id}\n\
         Deny: /workflow deny-workflow {run_id}\n\
         Continue after approval: /workflow resume --live {run_id}\n",
        inspection.workflow_name,
        inspection.phase_count,
        inspection.max_agents,
        inspection.max_parallelism,
        inspection.raw_script_path,
        inspection.compiled_spec_path,
        raw_script_preview,
        inspection.raw_script_path,
        approvals_path.display(),
        inspection.cost_warning
    )
}

fn render_denied_note(
    inspection: &WorkflowApprovalInspection,
    approvals_path: &Path,
    run_id: &str,
) -> String {
    format!(
        "Workflow denied and cancelled: {run_id}\n\
         Workflow: {}\n\
         Raw script: {}\n\
         Approval store: {}\n",
        inspection.workflow_name,
        inspection.raw_script_path,
        approvals_path.display()
    )
}

fn raw_script_preview(path: &str) -> String {
    let Ok(raw) = std::fs::read_to_string(path) else {
        return "  <unavailable>".to_string();
    };
    let mut lines = raw.lines().take(80).collect::<Vec<_>>();
    let truncated = raw.lines().count() > lines.len();
    if truncated {
        lines.push("...");
    }
    lines
        .into_iter()
        .map(|line| format!("  {line}"))
        .collect::<Vec<_>>()
        .join("\n")
}
