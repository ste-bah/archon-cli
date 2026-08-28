#[path = "workflow_decomposition_proof_support.rs"]
mod support;

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::time::Duration;

use support::*;

#[test]
fn external_runtime_inputs_are_exactly_the_five_generic_contract_names() {
    assert_eq!(
        BTreeSet::from([
            EXTERNAL_PRD_ENV,
            EXTERNAL_TASK_ROOT_ENV,
            PROTECTED_ROOT_ENV,
            EVIDENCE_ROOT_ENV,
            SYNTHETIC_CLEARANCE_ENV,
        ]),
        BTreeSet::from([
            "ARCHON_R2A_EXTERNAL_PRD",
            "ARCHON_R2A_EXTERNAL_TASK_ROOT",
            "ARCHON_R2A_PROTECTED_ROOT",
            "ARCHON_R2A_EVIDENCE_ROOT",
            "ARCHON_R2A_SYNTHETIC_CLEARANCE",
        ])
    );
}

#[test]
fn external_harness_source_has_no_implementation_action() {
    let source = include_str!("workflow_decomposition_external_prd_live.rs");
    let live = rust_function_body(source, "fn external_prd_decomposition_only_live()").unwrap();
    assert!(!live.contains("ImplementSynthetic"));
    assert!(!live.contains("workflow run"));
    assert!(!live.contains("git push"));
    assert!(!live.contains("kill"));
    assert!(!live.contains("ProofAction::Pause"));
    assert!(!live.contains("ProofAction::Resume"));
    assert!(live.contains("tui-pause-command.txt"));
    assert!(live.contains("tui-resume-command.txt"));
    assert!(live.contains("evaluate_preflight"));
    assert!(!live.contains("capture_status("), "{live}");
    assert!(live.contains("wait_for_owner_action"), "{live}");
    assert!(live.contains("capture_durable_status"), "{live}");
    for action in [
        ProofAction::Decompose {
            prd: "PRD.md".into(),
            tasks: "tasks/PRD-X".into(),
        },
        ProofAction::Status {
            run_id: "wf-x".into(),
        },
        ProofAction::Pause {
            run_id: "wf-x".into(),
        },
        ProofAction::Resume {
            run_id: "wf-x".into(),
        },
    ] {
        require_external_action(&action).unwrap();
    }
}

#[test]
#[ignore = "manual external R2a proof; requires synthetic clearance and operator TUI launch"]
fn external_prd_decomposition_only_live() {
    let source_project = Path::new(env!("CARGO_MANIFEST_DIR"));
    let prd = PathBuf::from(required_env(EXTERNAL_PRD_ENV));
    let task_root = PathBuf::from(required_env(EXTERNAL_TASK_ROOT_ENV));
    let project = common_existing_ancestor(&prd, &task_root).unwrap();
    let runtime = standard_runtime_binary(&project);
    let peer = standard_deployed_peer().unwrap();
    let protected_root = PathBuf::from(required_env(PROTECTED_ROOT_ENV));
    let evidence_root = PathBuf::from(required_env(EVIDENCE_ROOT_ENV)).join("external");
    let clearance_path = PathBuf::from(required_env(SYNTHETIC_CLEARANCE_ENV));

    assert!(prd.is_file(), "external PRD is missing: {}", prd.display());
    assert!(
        task_root.is_dir(),
        "external task root is missing: {}",
        task_root.display()
    );
    assert!(
        std::fs::read_dir(&task_root).unwrap().next().is_none(),
        "external proof requires a fresh empty task root"
    );
    assert!(
        !evidence_root.exists() || std::fs::read_dir(&evidence_root).unwrap().next().is_none(),
        "external evidence root must be fresh"
    );
    let processes = proof_process_inventory().unwrap();
    let source_head = source_revision(source_project).unwrap();
    let clearance = read_clearance(&clearance_path).unwrap();
    let manifest_root = clearance_path.parent().expect("clearance parent");
    validate_evidence_manifest(manifest_root, &clearance.evidence_manifest_digest).unwrap();
    let before_result = snapshot_tree(&protected_root);
    evaluate_preflight(&ProofPreflightFacts {
        runtime_exists: runtime.exists(),
        runtime_is_file: runtime.is_file(),
        target_is_fresh: std::fs::read_dir(&task_root).unwrap().next().is_none()
            && (!evidence_root.exists()
                || std::fs::read_dir(&evidence_root).unwrap().next().is_none()),
        observe_mode: require_observe_config(&project).is_ok(),
        active_work: processes.conflicts,
        idle_tui_count: processes.idle_tui_count,
        expected_idle_tui_count: 1,
        deployed_binaries_match: require_matching_binaries(&runtime, &peer).is_ok(),
        protected_snapshot_valid: before_result.is_ok(),
    })
    .unwrap();
    runtime_matches_clearance(&clearance, &source_head, &runtime).unwrap();
    let before = before_result.unwrap();
    std::fs::create_dir_all(&evidence_root).unwrap();
    write_json(&evidence_root.join("protected-before.json"), &before).unwrap();

    let existing = current_run_ids(&project).unwrap();
    let tui_command = format!(
        "/workflow decompose --prd {} --tasks {}",
        prd.display(),
        task_root.display()
    );
    println!("Launch this exact command in the already-open Archon TUI:\n{tui_command}");
    std::fs::write(evidence_root.join("tui-launch-command.txt"), &tui_command).unwrap();
    let run_id = wait_for_new_fixed_run(&project, &existing, Duration::from_secs(300)).unwrap();
    let identity =
        runtime_identity_from_fixed_run(source_head, &runtime, &project, &run_id).unwrap();
    require_clearance_identity(&clearance, &identity).unwrap();
    assert_fixed_identity_and_route(&project, &run_id);

    wait_for_event_line(
        &project,
        &run_id,
        &["author_attempt_started", "skeleton-author-1"],
        Duration::from_secs(1_500),
    )
    .unwrap();
    let subject = "skeleton".to_string();
    let before_attempt = fixed_attempt(&project, &run_id, &subject);
    let pause_command = format!("/workflow pause {run_id}");
    println!(
        "Run this exact command in the same Archon TUI:
{pause_command}"
    );
    std::fs::write(evidence_root.join("tui-pause-command.txt"), &pause_command).unwrap();
    wait_for_event_line(
        &project,
        &run_id,
        &["author_attempt_interrupted", "skeleton-author-1"],
        Duration::from_secs(120),
    )
    .unwrap();
    wait_for_event(&project, &run_id, "paused", Duration::from_secs(120)).unwrap();
    let status_command = format!("/workflow status {run_id}");
    println!(
        "Inspect paused status in the same Archon TUI:
{status_command}"
    );
    std::fs::write(
        evidence_root.join("tui-status-command.txt"),
        &status_command,
    )
    .unwrap();
    wait_for_owner_action(&project, &run_id, "status", Duration::from_secs(300)).unwrap();
    capture_durable_status(&project, &run_id, &evidence_root, "paused-status.json");
    let resume_command = format!("/workflow resume --live {run_id}");
    println!(
        "Run this exact command in the same Archon TUI after its retained worker has released ownership:
{resume_command}"
    );
    std::fs::write(
        evidence_root.join("tui-resume-command.txt"),
        &resume_command,
    )
    .unwrap();
    wait_for_event(&project, &run_id, "resumed", Duration::from_secs(300)).unwrap();
    assert_eq!(fixed_attempt(&project, &run_id, &subject), before_attempt);
    assert_acceptance_reused(&project, &run_id);

    let terminal = wait_for_terminal_run(&project, &run_id, Duration::from_secs(7_200)).unwrap();
    assert_eq!(terminal.status, archon_workflow::RunStatus::Completed);
    capture_durable_status(&project, &run_id, &evidence_root, "terminal-status.json");
    assert_decomposition_complete(&project, &run_id);
    assert_same_tui_owner(&project, &run_id);
    assert_eq!(
        collect_host_command_receipts(&project, &run_id, &evidence_root.join("run")).unwrap(),
        BTreeSet::from([
            "freeze-acceptance".to_string(),
            "freeze-skeleton".to_string(),
            "land-task-body".to_string(),
            "requirements-trace".to_string(),
            "task-set-lint".to_string(),
        ])
    );
    assert_no_implementation_run(&project, &existing, &run_id);
    copy_decomposition_log(&project, &run_id, &evidence_root.join("decompose.log")).unwrap();
    copy_evidence(&project, &run_id, &evidence_root.join("run")).unwrap();
    copy_external_artifacts(&project, &evidence_root).unwrap();
    let after = snapshot_tree(&protected_root).unwrap();
    write_json(&evidence_root.join("protected-after.json"), &after).unwrap();
    require_unchanged(&before, &after).unwrap();
    let manifest = build_evidence_manifest("external", identity, &evidence_root).unwrap();
    write_json(&evidence_root.join("manifest.json"), &manifest).unwrap();
}

fn assert_fixed_identity_and_route(project: &Path, run_id: &str) {
    let store = archon_workflow::WorkflowStore::project(project);
    let run_dir = store.run_dir(run_id);
    let state: serde_json::Value =
        serde_json::from_slice(&std::fs::read(run_dir.join("decomposition/state.json")).unwrap())
            .unwrap();
    assert_eq!(state["run_kind"], "fixed_decomposition_v1");
    assert_eq!(
        state["identity"]["template_version"],
        "fixed-decomposition-v1"
    );
    assert!(state["identity"]["script_digest"].as_str().is_some());
    assert!(state["identity"]["catalog_digest"].as_str().is_some());
    let route: serde_json::Value = serde_json::from_slice(
        &std::fs::read(run_dir.join("decomposition/provider-route.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(route["origin"], "trusted_config");
}

fn assert_acceptance_reused(project: &Path, run_id: &str) {
    let store = archon_workflow::WorkflowStore::project(project);
    let events = parse_json_lines(&store.events_path(run_id)).unwrap();
    assert!(events.iter().any(|event| {
        event["detail"]["phase"] == "acceptance" && event["detail"]["reused"] == true
    }));
}

fn fixed_attempt(project: &Path, run_id: &str, subject: &str) -> u64 {
    let store = archon_workflow::WorkflowStore::project(project);
    let state: serde_json::Value = serde_json::from_slice(
        &std::fs::read(store.run_dir(run_id).join("decomposition/state.json")).unwrap(),
    )
    .unwrap();
    state["attempts"][subject]["logical_attempt"]
        .as_u64()
        .expect("logical attempt")
}

fn wait_for_owner_action(
    project: &Path,
    run_id: &str,
    action: &str,
    timeout: Duration,
) -> Result<(), String> {
    let path = archon_workflow::WorkflowStore::project(project)
        .run_dir(run_id)
        .join("decomposition/interactive-owner.json");
    let deadline = std::time::Instant::now() + timeout;
    loop {
        if let Ok(bytes) = read_regular_nofollow(&path)
            && let Ok(record) = serde_json::from_slice::<serde_json::Value>(&bytes)
            && record["actions"]
                .as_array()
                .is_some_and(|actions| actions.iter().any(|value| value == action))
        {
            return Ok(());
        }
        if std::time::Instant::now() >= deadline {
            return Err(format!(
                "timed out waiting for retained-owner action {action} in {}",
                path.display()
            ));
        }
        std::thread::sleep(Duration::from_millis(250));
    }
}

fn capture_durable_status(project: &Path, run_id: &str, evidence_root: &Path, name: &str) {
    let store = archon_workflow::WorkflowStore::project(project);
    let run_dir = store.run_dir(run_id);
    let fixed: serde_json::Value = serde_json::from_slice(
        &read_regular_nofollow(&run_dir.join("decomposition/state.json")).unwrap(),
    )
    .unwrap();
    let owner: serde_json::Value = serde_json::from_slice(
        &read_regular_nofollow(&run_dir.join("decomposition/interactive-owner.json")).unwrap(),
    )
    .unwrap();
    let finalization_path = run_dir.join("v2/finalization.json");
    let finalization = read_regular_nofollow(&finalization_path)
        .ok()
        .and_then(|bytes| serde_json::from_slice::<serde_json::Value>(&bytes).ok());
    write_json(
        &evidence_root.join(name),
        &serde_json::json!({
            "run": store.load_state(run_id).unwrap(),
            "fixed": fixed,
            "owner": owner,
            "finalization": finalization,
        }),
    )
    .unwrap();
}

fn assert_decomposition_complete(project: &Path, run_id: &str) {
    let store = archon_workflow::WorkflowStore::project(project);
    let events = parse_json_lines(&store.events_path(run_id)).unwrap();
    assert!(
        events
            .iter()
            .any(|event| event["kind"] == "decomposition_completed")
    );
}

fn assert_same_tui_owner(project: &Path, run_id: &str) {
    let store = archon_workflow::WorkflowStore::project(project);
    let record: serde_json::Value = serde_json::from_slice(
        &std::fs::read(
            store
                .run_dir(run_id)
                .join("decomposition/interactive-owner.json"),
        )
        .unwrap(),
    )
    .unwrap();
    assert_eq!(record["schema_version"], 1);
    assert!(
        record["owner_identity"]
            .as_str()
            .is_some_and(|value| value.len() >= 16)
    );
    let actions = record["actions"]
        .as_array()
        .unwrap()
        .iter()
        .map(|value| value.as_str().unwrap().to_string())
        .collect::<BTreeSet<_>>();
    assert_eq!(
        actions,
        BTreeSet::from([
            "launch".to_string(),
            "pause".to_string(),
            "resume".to_string(),
            "status".to_string(),
        ])
    );
}

fn assert_no_implementation_run(project: &Path, before: &BTreeSet<String>, fixed_run_id: &str) {
    let runs = archon_workflow::WorkflowStore::project(project)
        .list_runs()
        .unwrap();
    let new = runs
        .into_iter()
        .filter(|run| !before.contains(&run.id))
        .collect::<Vec<_>>();
    assert_eq!(new.len(), 1, "external proof created an extra workflow run");
    assert_eq!(new[0].id, fixed_run_id);
}

fn copy_external_artifacts(project: &Path, evidence_root: &Path) -> Result<(), String> {
    for (source, name) in [(
        project.join(".archon/logs/workflow-gates-shadow.jsonl"),
        "workflow-gates-shadow.jsonl",
    )] {
        if source.exists() {
            std::fs::copy(&source, evidence_root.join(name))
                .map_err(|error| format!("copying {}: {error}", source.display()))?;
        }
    }
    Ok(())
}

fn required_env(name: &str) -> String {
    std::env::var(name).unwrap_or_else(|_| panic!("{name} is required"))
}
