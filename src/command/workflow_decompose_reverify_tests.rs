//! End-to-end tests for the reverify command. Expected state is recorded
//! before invoking the real entry point; assertions read its persisted output.
#[path = "workflow_decompose_reverify_fixture_repository.rs"]
mod fixture_repository;
use super::*;
use archon_workflow::{
    DecompositionPhase, FixedDecompositionStateV1, FixedRunIdentityV1, RunStatus, WorkflowRunKind,
    WorkflowV2CallRecord, WorkflowV2HostCall, WorkflowV2HostMethod, WorkflowV2HostOptions,
    WorkflowV2Result, WorkflowV2ResultStore, WorkflowV2Status, task_set_contract::content_digest,
};

#[test]
fn missing_gate_verdict_is_recorded_as_operational_failure_with_stderr() {
    // EXPECTED BEFORE ASSESSMENT: no envelope means no policy verdict; record
    // operational failure, exit cause and the captured stderr excerpt.
    let result = archon_workflow::HostCommandResult {
        exit_code: Some(127),
        stdout: String::new(),
        stderr: "archon: executable not found".into(),
        stdout_bytes: 0,
        stderr_bytes: 29,
        timed_out: false,
        interrupted: false,
        stdout_truncated: false,
        stderr_truncated: false,
        gate_envelope: None,
        publication_receipt: None,
        observed_only: false,
        subjects: Vec::new(),
        postcondition: None,
    };
    println!(
        "EVIDENCE reverify missing_verdict BEFORE: exit_code=127 envelope=absent stderr=archon: executable not found"
    );
    let (verdict, findings) = super::workflow_decompose_reverify::gate_assessment(&result);
    assert_eq!(verdict, "operational_failure");
    assert_eq!(findings.len(), 1);
    assert!(findings[0].contains("exit code 127"));
    assert!(findings[0].contains("archon: executable not found"));
    println!(
        "EVIDENCE reverify missing_verdict AFTER: verdict={} findings={} cause_and_stderr_preserved=true",
        verdict,
        findings.len()
    );
}

#[test]
fn gate_operational_error_preserves_kind_text_and_stderr() {
    let result = archon_workflow::HostCommandResult {
        exit_code: Some(1),
        stdout: String::new(),
        stderr: "judge transport closed".into(),
        stdout_bytes: 0,
        stderr_bytes: 22,
        timed_out: false,
        interrupted: false,
        stdout_truncated: false,
        stderr_truncated: false,
        gate_envelope: Some(archon_workflow::GateEnvelopeV1 {
            schema_version: 1,
            report: serde_json::Value::Null,
            policy_findings: Vec::new(),
            operational_error: Some(archon_workflow::GateOperationalError {
                kind: "transport".into(),
                text: "judge unavailable".into(),
            }),
        }),
        publication_receipt: None,
        observed_only: false,
        subjects: Vec::new(),
        postcondition: None,
    };

    println!(
        "EVIDENCE reverify gate_operational_error BEFORE: kind=transport text=judge unavailable stderr=judge transport closed"
    );
    let (verdict, findings) = super::workflow_decompose_reverify::gate_assessment(&result);
    assert_eq!(verdict, "operational_failure");
    assert_eq!(findings.len(), 1);
    assert!(findings[0].contains("transport"));
    assert!(findings[0].contains("judge unavailable"));
    assert!(findings[0].contains("judge transport closed"));
    println!(
        "EVIDENCE reverify gate_operational_error AFTER: verdict={verdict} cause_fields_and_stderr_preserved=true"
    );
}

#[test]
fn typed_input_invalid_gate_error_becomes_a_subject_finding() {
    // EXPECTED BEFORE ASSESSMENT: the gate has typed the judged body itself as
    // invalid, so reverify records its parse error as remediation rather than
    // treating an absent policy verdict as an operational failure.
    let result = archon_workflow::HostCommandResult {
        exit_code: Some(1),
        stdout: String::new(),
        stderr: "invalid task body".into(),
        stdout_bytes: 0,
        stderr_bytes: 18,
        timed_out: false,
        interrupted: false,
        stdout_truncated: false,
        stderr_truncated: false,
        gate_envelope: Some(archon_workflow::GateEnvelopeV1 {
            schema_version: 1,
            report: serde_json::Value::Null,
            policy_findings: Vec::new(),
            operational_error: Some(archon_workflow::GateOperationalError {
                kind: "input_invalid".into(),
                text: "task body has no YAML task block".into(),
            }),
        }),
        publication_receipt: None,
        observed_only: true,
        subjects: Vec::new(),
        postcondition: None,
    };
    println!(
        "EVIDENCE reverify input_invalid BEFORE: kind=input_invalid text=task body has no YAML task block"
    );
    let (verdict, findings) = super::workflow_decompose_reverify::gate_assessment(&result);
    assert_eq!(verdict, "needs_remediation");
    assert_eq!(findings, vec!["task body has no YAML task block"]);
    println!(
        "EVIDENCE reverify input_invalid AFTER: verdict={verdict} findings={}",
        findings.len()
    );
}

const PRD: &str = "# PRD X\n\n## Requirements\n| ID | Requirement |\n|---|---|\n| REQ-X-001 | Prove the fixture. |\n\n## Acceptance Criteria\n| ID | Criterion |\n|---|---|\n| AC-X-001 | The fixture is proven. |\n";
const PRIOR_ID: &str = "prior-final-report";

pub(super) struct Fixture {
    pub(super) _temp: tempfile::TempDir,
    pub(super) project: std::path::PathBuf,
    pub(super) store: archon_workflow::WorkflowStore,
    pub(super) run_id: String,
    pub(super) bodies: Vec<std::path::PathBuf>,
    pub(super) prior_record: std::path::PathBuf,
}

pub(super) fn fixture() -> Fixture {
    let temp = tempfile::tempdir().unwrap();
    let project = temp.path().canonicalize().unwrap();
    let prd = project.join("prds/PRD-X.md");
    let task_root = project.join("tasks/PRD-X");
    std::fs::create_dir_all(prd.parent().unwrap()).unwrap();
    std::fs::create_dir_all(&task_root).unwrap();
    std::fs::write(&prd, PRD).unwrap();
    assert!(
        std::process::Command::new("git")
            .args(["init", "-q"])
            .current_dir(&project)
            .status()
            .unwrap()
            .success()
    );
    let acceptance =
        crate::command::workflow_decompose_frozen_chain::test_support::freeze_acceptance(
            &project, &prd, &task_root,
        );
    crate::command::workflow_decompose_frozen_chain::test_support::freeze_skeleton(
        &project,
        &task_root,
        &acceptance,
        &["TASK-X-010", "TASK-X-020"],
    );
    let bodies = ["TASK-X-010.md", "TASK-X-020.md"].into_iter().map(|name| {
        let path = task_root.join(name);
        let task_id = name.trim_end_matches(".md");
        let body = format!("```yaml\ntask_id: {task_id}\ntitle: T\ncomplexity: small\nstatus: ready\ndepends_on: []\nblocks: []\nimplements: [AC-X-001]\nrequired_env_keys: []\nrequired_tools: []\ndeliverable_contracts: []\n```\n\n# {task_id}\n\nA complete implementation.\n");
        std::fs::write(&path, body).unwrap();
        path
    }).collect::<Vec<_>>();
    fixture_repository::record(&project, &task_root);
    let store = archon_workflow::WorkflowStore::project(&project);
    let run = store
        .create_run(WorkflowSpec {
            schema: archon_workflow::spec::WORKFLOW_SCHEMA.into(),
            name: "fixed-decomposition-fixture".into(),
            task: "fixture".into(),
            target_repository_root: Some(project.to_string_lossy().into_owned()),
            max_parallelism: 1,
            max_agents: 1,
            stages: Vec::new(),
            permissions: Default::default(),
            learning_hooks: Vec::new(),
        })
        .unwrap();
    let revision = env!("ARCHON_GIT_HASH");
    let catalog =
        super::super::workflow_host_command_catalog::fixed_decomposition_catalog(revision).unwrap();
    store
        .write_run_json(
            &run.id,
            "decomposition/state.json",
            &FixedDecompositionStateV1 {
                schema_version: 1,
                run_kind: WorkflowRunKind::FixedDecompositionV1,
                identity: FixedRunIdentityV1 {
                    template_version: "fixed-decomposition-v1".into(),
                    starting_binary_revision: revision.into(),
                    script_digest: archon_workflow::workflow_scaffold_hash(
                        super::super::workflow_decompose::FIXED_SCRIPT_SOURCE,
                    ),
                    catalog_digest: catalog.digest,
                    project_root_identity: project.to_string_lossy().into_owned(),
                    prd_identity: prd.to_string_lossy().into_owned(),
                    task_root_identity: task_root.to_string_lossy().into_owned(),
                },
                phase: DecompositionPhase::Completed,
                attempts: Default::default(),
                dispositions: Default::default(),
                log_path: store
                    .run_dir(&run.id)
                    .join("decomposition.log")
                    .to_string_lossy()
                    .into(),
            },
        )
        .unwrap();
    let mut completed = run;
    completed.status = RunStatus::Completed;
    completed.updated_at = chrono::Utc::now();
    std::fs::write(
        store.state_path(&completed.id),
        serde_json::to_vec_pretty(&completed).unwrap(),
    )
    .unwrap();
    let v2 = WorkflowV2ResultStore::new(store.run_dir(&completed.id).join("v2"));
    let mut result = WorkflowV2Result::default();
    result.status = WorkflowV2Status::Accepted;
    result.summary = "prior accepted report".into();
    result.data = serde_json::json!({"outcome":"accepted"});
    v2.save_call_record(&WorkflowV2CallRecord::new(
        &completed.id,
        WorkflowV2HostCall {
            id: PRIOR_ID.into(),
            method: WorkflowV2HostMethod::FinalReport,
            write_mode: None,
            options: WorkflowV2HostOptions::default(),
        },
        1,
        "prior-input".into(),
        result,
        Vec::new(),
    ))
    .unwrap();
    let prior_record = v2.result_path(PRIOR_ID);
    Fixture {
        _temp: temp,
        project,
        store,
        run_id: completed.id,
        bodies,
        prior_record,
    }
}

pub(super) fn archon_binary() -> std::path::PathBuf {
    let binary = std::env::var_os("CARGO_BIN_EXE_archon")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| {
            std::env::current_exe()
                .unwrap()
                .parent()
                .unwrap()
                .parent()
                .unwrap()
                .join(format!("archon{}", std::env::consts::EXE_SUFFIX))
        });
    assert!(
        binary.is_file(),
        "built archon executable missing: {}",
        binary.display()
    );
    binary
}

pub(super) fn invoke_reverify(binary: &std::path::Path, fixture: &Fixture) -> std::process::Output {
    std::process::Command::new(binary)
        .args(["workflow", "reverify", &fixture.run_id])
        .current_dir(&fixture.project)
        .env("ARCHON_LOG_DIR", fixture.project.join(".test-logs"))
        .env("XDG_DATA_HOME", fixture.project.join(".test-data"))
        .env("HOME", &fixture.project)
        .output()
        .unwrap()
}

#[tokio::test]
async fn operational_reverify_supersedes_stale_report_and_preserves_bodies() {
    // EXPECTED BEFORE RUN: without configured provider credentials the real
    // fidelity gate records operational_failure and supersedes the old report,
    // while preserving run state and bytes, creating no ledger, and exiting
    // non-zero with the cause and instruction to reverify again.
    let fixture = fixture();
    let before = fixture
        .bodies
        .iter()
        .map(|p| std::fs::read(p).unwrap())
        .collect::<Vec<_>>();
    let run_state_path = fixture.store.run_dir(&fixture.run_id).join("state.json");
    let run_state_before = std::fs::read(&run_state_path).unwrap();
    let run_before: serde_json::Value = serde_json::from_slice(&run_state_before).unwrap();
    assert_eq!(run_before["status"], "completed");
    let prior_bytes = std::fs::read(&fixture.prior_record).unwrap();
    let prior: WorkflowV2CallRecord = serde_json::from_slice(&prior_bytes).unwrap();
    println!(
        "EVIDENCE reverify stale_report BEFORE: prior_outcome=accepted bodies={} run=Completed provider_credentials=absent",
        before.len()
    );
    let binary = archon_binary();
    let output = invoke_reverify(&binary, &fixture);
    assert!(
        !output.status.success(),
        "operational failure must exit non-zero"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("operational_failure"), "{stderr}");
    assert!(stderr.contains("reverify again"), "{stderr}");
    let output_text = String::from_utf8(output.stdout).unwrap();
    let v2 = WorkflowV2ResultStore::new(fixture.store.run_dir(&fixture.run_id).join("v2"));
    let record = v2
        .load_call_records()
        .unwrap()
        .into_iter()
        .filter(|r| {
            r.call.id.contains("reverify-") && r.call.method == WorkflowV2HostMethod::FinalReport
        })
        .max_by(|a, b| a.finished_at.cmp(&b.finished_at))
        .unwrap();
    let path = fixture.store.run_dir(&fixture.run_id).join(format!(
        "decomposition/reverify/reports/{}.json",
        record.call.id
    ));
    let report_bytes = std::fs::read(path).unwrap();
    let report: serde_json::Value = serde_json::from_slice(&report_bytes).unwrap();
    assert_eq!(
        report["outcome"], "operational_failure",
        "{output_text} report={report:#}"
    );
    assert_eq!(report["supersedes_id"], PRIOR_ID);
    assert_eq!(report["supersedes_digest"], prior.output_hash);
    for (subject, path) in report["subjects"]
        .as_array()
        .unwrap()
        .iter()
        .zip(&fixture.bodies)
    {
        let disk = std::fs::read(path).unwrap();
        assert_eq!(subject["disk_digest"], content_digest(&disk));
        assert_eq!(subject["judged_digest"], content_digest(&disk));
    }
    for gate in report["set_gates"].as_array().unwrap() {
        assert!(gate["logic_version"].as_u64().is_some());
        assert!(gate["logic_digest"].as_str().is_some());
        assert!(gate["build"].as_str().is_some());
    }
    assert_eq!(
        fixture
            .bodies
            .iter()
            .map(|p| std::fs::read(p).unwrap())
            .collect::<Vec<_>>(),
        before
    );
    assert_eq!(std::fs::read(&fixture.prior_record).unwrap(), prior_bytes);
    assert_eq!(std::fs::read(&run_state_path).unwrap(), run_state_before);
    assert_eq!(
        fixture.store.load_state(&fixture.run_id).unwrap().status,
        RunStatus::Completed
    );
    let reverify_dir = fixture
        .store
        .run_dir(&fixture.run_id)
        .join("decomposition/reverify");
    assert!(!std::fs::read_dir(&reverify_dir).unwrap().any(|entry| {
        entry
            .unwrap()
            .file_name()
            .to_string_lossy()
            .starts_with("remediation-")
    }));
    println!(
        "EVIDENCE reverify stale_report AFTER: outcome={} supersedes={} bodies_unchanged=true run=Completed run_record_bytes_unchanged=true ledger_present=false exit_nonzero=true cause_and_retry_printed=true",
        report["outcome"], report["supersedes_id"]
    );
}

#[tokio::test]
async fn defective_body_is_a_finding_that_pauses_and_creates_a_ledger() {
    // EXPECTED BEFORE RUN: a body the gate cannot parse is a finding. Reverify
    // persists a needs_remediation report and ledger, pauses the run, keeps
    // body bytes unchanged, and exits successfully with a resume instruction.
    let fixture = fixture();
    let defective = b"not a task body\n".to_vec();
    std::fs::write(&fixture.bodies[0], &defective).unwrap();
    let _accepted_body = std::fs::read(&fixture.bodies[1]).unwrap();
    let state_path = fixture.store.run_dir(&fixture.run_id).join("state.json");
    let state_before = std::fs::read(&state_path).unwrap();
    println!(
        "EVIDENCE reverify defective_body BEFORE: task={} bytes={} run=Completed ledger_present=false",
        "TASK-X-010",
        defective.len()
    );
    let binary = archon_binary();
    let output = invoke_reverify(&binary, &fixture);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let v2 = WorkflowV2ResultStore::new(fixture.store.run_dir(&fixture.run_id).join("v2"));
    let latest = super::super::workflow_decompose_latest_report::latest_report(
        &fixture.store,
        &fixture.run_id,
    )
    .unwrap()
    .unwrap();
    let report_record = v2
        .load_call_records()
        .unwrap()
        .into_iter()
        .find(|record| record.call.id == latest.id)
        .unwrap();
    let report_path = fixture.store.run_dir(&fixture.run_id).join(format!(
        "decomposition/reverify/reports/{}.json",
        report_record.call.id
    ));
    let report: serde_json::Value =
        serde_json::from_slice(&std::fs::read(report_path).unwrap()).unwrap();
    let subject = report["subjects"]
        .as_array()
        .unwrap()
        .iter()
        .find(|subject| subject["task_id"] == "TASK-X-010")
        .unwrap();
    assert_eq!(report["outcome"], "needs_remediation");
    assert_eq!(subject["verdict"], "needs_remediation");
    assert!(!subject["findings"].as_array().unwrap().is_empty());
    assert!(
        subject["findings"][0]
            .as_str()
            .unwrap()
            .contains("task body")
    );
    assert_ne!(std::fs::read(&state_path).unwrap(), state_before);
    assert_eq!(
        fixture.store.load_state(&fixture.run_id).unwrap().status,
        RunStatus::Paused
    );
    let ledger = fixture.store.run_dir(&fixture.run_id).join(format!(
        "decomposition/reverify/remediation-{}.json",
        report["id"].as_str().unwrap()
    ));
    let ledger_value: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&ledger).unwrap()).unwrap();
    assert_eq!(ledger_value["report_id"], report["id"]);
    let ledger_subjects = ledger_value["subjects"].as_array().unwrap();
    let unique_subjects = ledger_subjects
        .iter()
        .map(|subject| subject.as_str().unwrap())
        .collect::<std::collections::HashSet<_>>();
    assert_eq!(unique_subjects.len(), ledger_subjects.len());
    let status = super::super::workflow_decompose_status::render(&fixture.store, &fixture.run_id)
        .unwrap()
        .unwrap();
    assert!(status.contains(&format!(
        "latest_report: id={} outcome=needs_remediation",
        report["id"].as_str().unwrap()
    )));
    assert!(status.contains(&format!(
        "resume_command: archon workflow resume --live --yes {}",
        fixture.run_id
    )));
    assert!(
        ledger_value["subjects"]
            .as_array()
            .unwrap()
            .iter()
            .any(|id| id == "TASK-X-010")
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("needs_remediation"), "{stdout}");
    assert!(stdout.contains("Resume:"), "{stdout}");
    assert_eq!(std::fs::read(&fixture.bodies[0]).unwrap(), defective);
    println!(
        "EVIDENCE reverify defective_body AFTER: outcome={} verdict={} findings={} run=Paused ledger_present=true exit_zero=true resume_printed=true body_unchanged=true",
        report["outcome"],
        subject["verdict"],
        subject["findings"].as_array().unwrap().len()
    );
}
