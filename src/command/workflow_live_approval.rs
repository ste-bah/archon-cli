use std::path::Path;

use anyhow::Result;
use archon_tui::app::TuiEvent;
use archon_tui::event_channel::TuiEventSender;
use archon_workflow::{
    LifecycleAction, LifecycleController, WorkflowApprovalDecision, WorkflowApprovalInspection,
    WorkflowApprovalRecord, WorkflowApprovalStore, WorkflowBundle, WorkflowBundleOrigin,
    WorkflowRun, WorkflowStore,
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
    let generated_v2 = generated_v2_origin(record.origin.as_ref());
    let count_label = if generated_v2 {
        "Dynamic host calls"
    } else {
        "Phases"
    };
    let write_label = if generated_v2 {
        "Write-capable host calls"
    } else {
        "Write-capable stages"
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
    let generated_config = generated_config_line(&record.raw_script_path);
    format!(
        "Workflow approval recorded: {decision}\n\
         Workflow: {}\n\
         {count_label}: {} | max agents: {} | max parallelism: {}\n\
         {generated_config}\
         Approval subject: {}\n\
         {write_label}: {write_stages}\n\
         External requirements: {external}\n\
         Raw script: {}\n\
         Approval store: {}\n\
         Cost/rate-limit: live provider token and rate limits apply.\n",
        record.workflow_name,
        record.phase_count,
        record.max_agents,
        record.max_parallelism,
        record_hash_summary(
            &record.workflow_hash,
            &record.compiled_hash,
            record.generated_metadata_hash.as_deref(),
            &record.approval_subject_hash
        ),
        record.raw_script_path,
        approvals_path.display()
    )
}

fn render_approval_request(
    inspection: &WorkflowApprovalInspection,
    approvals_path: &Path,
    run_id: &str,
) -> String {
    let generated_v2 = generated_v2_origin(inspection.origin.as_ref());
    let count_label = if generated_v2 {
        "Dynamic host calls"
    } else {
        "Phases"
    };
    let write_label = if generated_v2 {
        "Write-capable host calls"
    } else {
        "Write-capable stages"
    };
    let compiled_label = if generated_v2 {
        "Compiled metadata"
    } else {
        "Compiled spec"
    };
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
    let generated_config = generated_config_line(&inspection.raw_script_path);
    format!(
        "Workflow awaiting approval: {run_id}\n\
         Workflow: {}\n\
         {count_label}: {} | max agents: {} | max parallelism: {}\n\
         {generated_config}\
         Approval subject: {}\n\
         {write_label}: {write_stages}\n\
         External requirements: {external}\n\
         Raw script: {}\n\
         {compiled_label}: {}\n\
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
        inspection_hash_summary(inspection),
        inspection.raw_script_path,
        inspection.compiled_spec_path,
        raw_script_preview,
        inspection.raw_script_path,
        approvals_path.display(),
        inspection.cost_warning
    )
}

fn generated_v2_origin(origin: Option<&WorkflowBundleOrigin>) -> bool {
    matches!(
        origin,
        Some(WorkflowBundleOrigin::GeneratedHarness | WorkflowBundleOrigin::SavedCommand)
    )
}

fn generated_config_line(raw_script_path: &str) -> String {
    let metadata_path = Path::new(raw_script_path)
        .parent()
        .map(|parent| parent.join("v2/generated-metadata.json"));
    let Some(metadata_path) = metadata_path else {
        return String::new();
    };
    let Ok(raw) = std::fs::read_to_string(metadata_path) else {
        return String::new();
    };
    let Ok(value) = serde_json::from_str::<serde_json::Value>(&raw) else {
        return String::new();
    };
    let Some(generated) = value.get("generated_config") else {
        return String::new();
    };
    let repair = generated
        .get("max_repair_iterations")
        .and_then(serde_json::Value::as_u64);
    let investigation = generated
        .get("max_investigation_iterations")
        .and_then(serde_json::Value::as_u64);
    let verification_timeout = generated
        .get("verification_branch_timeout_secs")
        .and_then(serde_json::Value::as_u64);
    let host_timeout = generated
        .get("host_call_timeout_secs")
        .and_then(serde_json::Value::as_u64);
    match (repair, investigation) {
        (Some(repair), Some(investigation)) => {
            let timeout_note = match (verification_timeout, host_timeout) {
                (Some(verification_timeout), Some(host_timeout)) => format!(
                    " | verification_branch_timeout_secs={verification_timeout} | host_call_timeout_secs={host_timeout}"
                ),
                _ => String::new(),
            };
            format!(
                "Generated caps: repair_iterations={repair} | investigation_iterations={investigation}{timeout_note}\n"
            )
        }
        _ => String::new(),
    }
}

fn render_denied_note(
    inspection: &WorkflowApprovalInspection,
    approvals_path: &Path,
    run_id: &str,
) -> String {
    format!(
        "Workflow denied and cancelled for matching approval subject: {run_id}\n\
         Workflow: {}\n\
         Approval subject: {}\n\
         Raw script: {}\n\
         Approval store: {}\n",
        inspection.workflow_name,
        inspection_hash_summary(inspection),
        inspection.raw_script_path,
        approvals_path.display()
    )
}

fn inspection_hash_summary(inspection: &WorkflowApprovalInspection) -> String {
    record_hash_summary(
        &inspection.workflow_hash,
        &inspection.compiled_hash,
        inspection.generated_metadata_hash.as_deref(),
        &inspection.approval_subject_hash,
    )
}

fn record_hash_summary(
    workflow_hash: &str,
    compiled_hash: &str,
    generated_metadata_hash: Option<&str>,
    approval_subject_hash: &str,
) -> String {
    let generated = generated_metadata_hash
        .map(short_hash)
        .unwrap_or_else(|| "none".to_string());
    format!(
        "subject={} script={} compiled={} generated_metadata={}",
        short_hash(approval_subject_hash),
        short_hash(workflow_hash),
        short_hash(compiled_hash),
        generated
    )
}

fn short_hash(hash: &str) -> String {
    hash.chars().take(12).collect()
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
