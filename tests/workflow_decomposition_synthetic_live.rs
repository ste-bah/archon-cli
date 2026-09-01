#[path = "workflow_decomposition_proof_support.rs"]
mod support;

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::time::Duration;

use support::*;

const FIXTURE_PRD: &str = include_str!("fixtures/decomposition-synthetic/prd.md");
const TEMPLATE_ROOT: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/fixtures/decomposition-synthetic/project-template"
);

fn synthetic_floor() -> archon_workflow::task_universe::WorkflowV2DeliverableContract {
    archon_workflow::task_universe::WorkflowV2DeliverableContract {
        kind: "synthetic_observer_target".into(),
        artifact_path: ".archon/proof/synthetic-observer-target.json".into(),
        artifact_format: Some("json".into()),
        required_true_fields: vec!["ready".into()],
        ..Default::default()
    }
}

#[test]
fn synthetic_fixture_has_exact_obligations_and_commandless_floor() {
    let obligations = archon_workflow::obligation_ids::obligation_ids(FIXTURE_PRD);
    assert_eq!(
        obligations,
        BTreeSet::from([
            "AC-SYN-001".to_string(),
            "REQ-SYN-001".to_string(),
            "REQ-SYN-002".to_string(),
        ])
    );
    let acceptance = archon_workflow::obligation_ids::acceptance_criteria(FIXTURE_PRD);
    assert_eq!(acceptance.len(), 1);
    let criterion = acceptance.get("AC-SYN-001").unwrap();
    let floor = synthetic_floor();
    let encoded = serde_json::to_value(&floor).unwrap();
    assert!(criterion.contains(&floor.kind));
    assert!(criterion.contains(&floor.artifact_path));
    assert!(criterion.contains("required_true_fields"));
    assert!(criterion.contains("no `typed_verifier_command`"));
    assert!(encoded.get("typed_verifier_command").is_none());
    assert_eq!(
        archon_workflow::evaluate_declarative_floor(
            &floor,
            &archon_workflow::DeclarativeFloorFacts {
                artifact_present: false,
                artifact_byte_len: 0,
                artifact_json: None,
                registry_json: None,
                instance_count: 0,
            }
        ),
        archon_workflow::DeclarativeFloorEvaluation::Failed {
            findings: vec![
                "declared deliverable missing or empty: .archon/proof/synthetic-observer-target.json"
                    .into()
            ]
        }
    );
}

#[test]
fn synthetic_template_is_observe_only_and_scratch_bounded() {
    let root = Path::new(TEMPLATE_ROOT);
    require_observe_config(root).unwrap();
    assert!(root.join("tasks/PRD-SYNTHETIC/.gitkeep").exists());
    assert!(root.join("src/.gitkeep").exists());
    assert!(
        !root
            .join(".archon/proof/synthetic-observer-target.json")
            .exists()
    );
}

#[test]
fn decomposition_identity_cli_is_read_only_json() {
    let project = tempfile::tempdir().unwrap();
    let binary = Path::new(env!("CARGO_BIN_EXE_archon"));
    let output = std::process::Command::new(binary)
        .current_dir(project.path())
        .args(["workflow", "decomposition-identity"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let identity: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(identity["template_version"], "fixed-decomposition-v1");
    assert!(identity["script_digest"].as_str().is_some());
    assert!(identity["catalog_digest"].as_str().is_some());
    assert!(!project.path().join(".archon/workflows").exists());
}

#[test]
fn synthetic_live_path_calls_preflight_evaluator() {
    let source = include_str!("workflow_decomposition_synthetic_live.rs");
    let live = rust_function_body(source, "fn synthetic_full_lifecycle_live()").unwrap();
    assert!(live.contains("evaluate_preflight"));
}

#[test]
#[ignore = "manual live R2a proof; requires reviewed deployed binaries and configured provider"]
fn synthetic_full_lifecycle_live() {
    let source_project = Path::new(env!("CARGO_MANIFEST_DIR"));
    let deployed_project = source_project
        .parent()
        .expect("source checkout parent")
        .join("project-1");
    let runtime = standard_runtime_binary(&deployed_project);
    let peer = standard_deployed_peer();
    let expected_head = source_revision(source_project).unwrap();
    let processes = proof_process_inventory().unwrap();
    let work = tempfile::tempdir().expect("synthetic scratch project");
    copy_tree(Path::new(TEMPLATE_ROOT), work.path());
    std::fs::create_dir_all(work.path().join("prds")).unwrap();
    let prd_path = work
        .path()
        .join("prds")
        .join(format!("PRD-SYNTHETIC-{PROOF_PROMPT_CANARY}.md"));
    std::fs::write(&prd_path, FIXTURE_PRD).unwrap();
    let _ = std::fs::remove_file(work.path().join("tasks/PRD-SYNTHETIC/.gitkeep"));
    inherit_provider_config(&deployed_project, work.path()).unwrap();
    git_init(work.path());
    require_observe_config(work.path()).unwrap();

    let evidence_root = PathBuf::from(required_env(EVIDENCE_ROOT_ENV)).join("synthetic");
    let evidence_fresh =
        !evidence_root.exists() || std::fs::read_dir(&evidence_root).unwrap().next().is_none();
    evaluate_preflight(&ProofPreflightFacts {
        runtime_exists: runtime.exists(),
        runtime_is_file: runtime.is_file(),
        target_is_fresh: evidence_fresh,
        observe_mode: require_observe_config(work.path()).is_ok(),
        active_work: processes.conflicts,
        idle_tui_count: processes.idle_tui_count,
        expected_idle_tui_count: 0,
        deployed_binaries_match: deployed_binaries_agree(&runtime, peer.as_deref()),
        protected_snapshot_valid: true,
    })
    .unwrap();
    assert_fresh_evidence_root(&evidence_root);
    std::fs::create_dir_all(&evidence_root).unwrap();
    write_json(
        &evidence_root.join("floor-exemplar.json"),
        &synthetic_floor(),
    )
    .unwrap();

    let decompose = ProofAction::Decompose {
        prd: prd_path,
        tasks: work.path().join("tasks/PRD-SYNTHETIC"),
    };
    let existing = current_run_ids(work.path()).unwrap();
    let mut decomposition = spawn_action(
        &runtime,
        work.path(),
        &decompose,
        &evidence_root.join("decomposition-stdout.txt"),
        &evidence_root.join("decomposition-stderr.txt"),
    )
    .unwrap();
    let run_id = wait_for_new_fixed_run(work.path(), &existing, Duration::from_secs(30)).unwrap();
    wait_for_event_line(
        work.path(),
        &run_id,
        &["author_attempt_started", "skeleton-author-1"],
        Duration::from_secs(1_500),
    )
    .unwrap();
    let pause = command_output(
        &runtime,
        work.path(),
        &ProofAction::Pause {
            run_id: run_id.clone(),
        },
    )
    .unwrap();
    require_success(&pause, "synthetic pause").unwrap();
    wait_for_event_line(
        work.path(),
        &run_id,
        &["author_attempt_interrupted", "skeleton-author-1"],
        Duration::from_secs(120),
    )
    .unwrap();
    let paused_status = command_output(
        &runtime,
        work.path(),
        &ProofAction::Status {
            run_id: run_id.clone(),
        },
    )
    .unwrap();
    std::fs::write(
        evidence_root.join("paused-status.txt"),
        require_success(&paused_status, "synthetic paused status").unwrap(),
    )
    .unwrap();
    let child_status = wait_for_child(&mut decomposition, Duration::from_secs(120)).unwrap();
    assert!(
        child_status.success(),
        "paused decomposition child: {child_status}"
    );
    let paused_attempt = fixed_attempt(work.path(), &run_id, "skeleton");
    let resume = command_output(
        &runtime,
        work.path(),
        &ProofAction::Resume {
            run_id: run_id.clone(),
        },
    )
    .unwrap();
    std::fs::write(
        evidence_root.join("resume-output.txt"),
        require_success(&resume, "synthetic resume").unwrap(),
    )
    .unwrap();
    assert_interrupted_attempt_resumed(work.path(), &run_id, "skeleton", paused_attempt);
    assert_acceptance_reused(work.path(), &run_id);
    let identity =
        runtime_identity_from_fixed_run(expected_head.clone(), &runtime, work.path(), &run_id)
            .unwrap();
    assert_fixed_provider_route(work.path(), &run_id);
    assert_decomposition_events(work.path(), &run_id);
    assert_frozen_floor(work.path(), &run_id, &synthetic_floor());
    assert_candidate_canary_crossed_model_boundary(work.path());
    assert_eq!(
        collect_host_command_receipts(work.path(), &run_id, &evidence_root.join("decomposition"),)
            .unwrap(),
        BTreeSet::from([
            "freeze-acceptance".to_string(),
            "freeze-skeleton".to_string(),
            "land-task-body".to_string(),
            "requirements-trace".to_string(),
            "task-set-lint".to_string(),
        ])
    );
    let live_log = fixed_log_path(work.path(), &run_id);
    assert_log_canaries_absent(&live_log);
    let copied_log = evidence_root.join("decomposition/decompose.log");
    copy_decomposition_log(work.path(), &run_id, &copied_log).unwrap();
    assert_log_canaries_absent(&copied_log);
    copy_evidence(work.path(), &run_id, &evidence_root.join("decomposition")).unwrap();

    let implementation = ProofAction::ImplementSynthetic {
        tasks: work.path().join("tasks/PRD-SYNTHETIC"),
    };
    let before = current_run_ids(work.path()).unwrap();
    let output = command_output(&runtime, work.path(), &implementation).unwrap();
    let implementation_text = require_success(&output, "synthetic implementation").unwrap();
    std::fs::write(
        evidence_root.join("implementation-output.txt"),
        implementation_text,
    )
    .unwrap();
    let implementation_run = newest_run_not_in(work.path(), &before);
    let terminal =
        wait_for_terminal_run(work.path(), &implementation_run, Duration::from_secs(30)).unwrap();
    assert_eq!(terminal.status, archon_workflow::RunStatus::Completed);
    assert_synthetic_outputs(work.path());
    assert_observer_after_terminal(work.path(), &implementation_run);
    copy_evidence(
        work.path(),
        &implementation_run,
        &evidence_root.join("implementation"),
    )
    .unwrap();

    write_json(&evidence_root.join("identity.json"), &identity).unwrap();
    let manifest = build_evidence_manifest("synthetic", identity, &evidence_root).unwrap();
    write_json(&evidence_root.join("manifest.json"), &manifest).unwrap();
    std::fs::write(
        evidence_root.join("authority-probe-command.txt"),
        "run exact root test filter global_enforce_mode_cannot_promote_observer_authority against the deployed commit and capture its successful output in authority-probe.txt\n",
    )
    .unwrap();
}

#[test]
fn synthetic_clearance_gate_validates_review_artifact_call_site() {
    let source = include_str!("workflow_decomposition_synthetic_live.rs");
    let signature = concat!("fn synthetic_clearance_", "requires_probe_and_review()");
    let body = rust_function_body(source, signature).unwrap();
    assert!(
        body.contains("validate_independent_review_artifact"),
        "{body}"
    );
}

#[test]
#[ignore = "manual clearance gate; requires authority probe evidence and approved independent review"]
fn synthetic_clearance_requires_probe_and_review() {
    let evidence_root = PathBuf::from(required_env(EVIDENCE_ROOT_ENV)).join("synthetic");
    let identity: RuntimeIdentity = serde_json::from_slice(
        &std::fs::read(evidence_root.join("identity.json")).expect("synthetic identity evidence"),
    )
    .unwrap();
    let runtime_digest = identity_digest(&identity).unwrap();
    let probe: AuthorityProbeReceipt = serde_json::from_slice(
        &std::fs::read(evidence_root.join("authority-probe.json"))
            .expect("authority probe receipt"),
    )
    .unwrap();
    let probe_output =
        std::fs::read(evidence_root.join("authority-probe.txt")).expect("authority probe output");
    assert_eq!(probe.schema_version, 1);
    assert_eq!(
        probe.test_filter,
        "global_enforce_mode_cannot_promote_observer_authority"
    );
    assert_eq!(probe.runtime_identity_digest, runtime_digest);
    assert_eq!(probe.exit_code, 0);
    assert_eq!(probe.output_sha256, bytes_sha256(&probe_output));

    let review: IndependentReviewReceipt = serde_json::from_slice(
        &std::fs::read(evidence_root.join("review.json")).expect("independent review evidence"),
    )
    .unwrap();
    validate_independent_review_artifact(&evidence_root, &review).unwrap();
    let pre_review = build_pre_review_manifest(
        "synthetic",
        identity.clone(),
        &evidence_root,
        &review.artifact_path,
    )
    .unwrap();
    assert_eq!(review.schema_version, 1);
    assert!(!review.reviewer.trim().is_empty());
    assert_ne!(review.reviewer.trim().to_ascii_lowercase(), "self");
    assert_eq!(review.evidence_manifest_digest, pre_review.digest);
    assert_eq!(review.runtime_identity_digest, runtime_digest);
    assert!(review.approved);
    assert_eq!(review.unresolved_critical, 0);
    assert_eq!(review.unresolved_important, 0);

    let manifest = build_evidence_manifest("synthetic", identity.clone(), &evidence_root).unwrap();
    write_json(&evidence_root.join("manifest.json"), &manifest).unwrap();
    let clearance = SyntheticClearance {
        schema_version: 1,
        identity,
        fixture_digest: synthetic_fixture_digest(),
        evidence_manifest_digest: manifest.digest,
    };
    write_json(&evidence_root.join("clearance.json"), &clearance).unwrap();
}

#[path = "support/workflow_decomposition_synthetic_evidence.rs"]
mod synthetic_evidence;
use synthetic_evidence::*;

fn fixed_log_path(project: &Path, run_id: &str) -> PathBuf {
    let store = archon_workflow::WorkflowStore::project(project);
    let state: serde_json::Value = serde_json::from_slice(
        &std::fs::read(store.run_dir(run_id).join("decomposition/state.json")).unwrap(),
    )
    .unwrap();
    PathBuf::from(state["log_path"].as_str().unwrap())
}

fn assert_candidate_canary_crossed_model_boundary(project: &Path) {
    let task_root = project.join("tasks/PRD-SYNTHETIC");
    let bodies = std::fs::read_dir(task_root)
        .unwrap()
        .filter_map(Result::ok)
        .filter(|entry| entry.path().extension().and_then(|value| value.to_str()) == Some("md"))
        .map(|entry| std::fs::read_to_string(entry.path()).unwrap())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(bodies.contains(PROOF_CANDIDATE_CANARY), "{bodies}");
}

fn assert_log_canaries_absent(path: &Path) {
    let text = std::fs::read_to_string(path).unwrap();
    for canary in [
        PROOF_PROMPT_CANARY,
        PROOF_CANDIDATE_CANARY,
        PROOF_ENV_CANARY_VALUE,
        PROOF_SECRET_CANARY_VALUE,
    ] {
        assert!(
            !text.contains(canary),
            "{canary} leaked into {}",
            path.display()
        );
    }
}

fn required_env(name: &str) -> String {
    std::env::var(name).unwrap_or_else(|_| panic!("{name} is required"))
}

fn assert_fresh_evidence_root(path: &Path) {
    assert!(
        !path.exists() || std::fs::read_dir(path).unwrap().next().is_none(),
        "evidence root must be fresh: {}",
        path.display()
    );
}

fn newest_run_not_in(project: &Path, before: &BTreeSet<String>) -> String {
    archon_workflow::WorkflowStore::project(project)
        .list_runs()
        .unwrap()
        .into_iter()
        .find(|run| !before.contains(&run.id))
        .map(|run| run.id)
        .expect("new workflow run")
}

fn copy_tree(source: &Path, destination: &Path) {
    for entry in std::fs::read_dir(source).unwrap() {
        let entry = entry.unwrap();
        let target = destination.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            std::fs::create_dir_all(&target).unwrap();
            copy_tree(&entry.path(), &target);
        } else {
            std::fs::copy(entry.path(), target).unwrap();
        }
    }
}

fn git_init(root: &Path) {
    for args in [
        vec!["init", "-q"],
        vec!["config", "user.email", "proof@example.invalid"],
        vec!["config", "user.name", "R2a Proof"],
        vec!["add", "."],
        vec!["commit", "-qm", "synthetic baseline"],
    ] {
        let output = std::process::Command::new("git")
            .current_dir(root)
            .args(args)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
}
