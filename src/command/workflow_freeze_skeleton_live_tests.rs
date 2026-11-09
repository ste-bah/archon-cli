use super::*;
use archon_workflow::baseline_results::{BaselineOutcome, BaselineResults};
use archon_workflow::requirement_state::RequirementState;
use archon_workflow::task_set_contract::{ACCEPTANCE_LOCK_FILE, AcceptanceCheck};
#[path = "workflow_freeze_skeleton_live_server_tests.rs"]
mod server;

#[test]
fn skeleton_live_entry_publishes_corpus_decisions_and_never_schedules_done() {
    const CHILD: &str = "ARCHON_TEST_SKELETON_LIVE_CHILD";
    if std::env::var_os(CHILD).is_none() {
        let test = std::thread::current().name().unwrap().to_string();
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", &test, "--nocapture"])
            .env(CHILD, "1")
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        eprint!("{}", String::from_utf8_lossy(&output.stderr));
        return;
    }
    let temp = tempfile::tempdir().unwrap();
    let (tasks, mut pin) = seed_frozen_acceptance(&temp);
    std::fs::write(
        temp.path().join("value"),
        "value() { printf good; }\nvalue\n",
    )
    .unwrap();
    std::fs::write(temp.path().join("note"), "record\n").unwrap();
    std::fs::write(
        temp.path().join("other"),
        "value() { printf good; }\nvalue\n",
    )
    .unwrap();
    crate::command::workflow_task_set::passability_tests_support::commit_project(temp.path());
    let output = std::process::Command::new("git")
        .args(["rev-parse", "HEAD"])
        .current_dir(temp.path())
        .output()
        .unwrap();
    assert!(output.status.success());
    let base = String::from_utf8(output.stdout).unwrap().trim().to_string();
    let prd = temp.path().join("prds/PRD-X.md");
    let text = "## Acceptance Criteria\n| ID | Criterion |\n|---|---|\n| AC-X-001 | It keeps the good value for every input. |\n\n- REQ-VALUE-001: It keeps the good value for every input.\n- REQ-VALUE-002: It keeps the good value for every input.\n- REQ-VALUE-003: It keeps the good value for every input.\n";
    std::fs::write(&prd, text).unwrap();
    let mut contract: AcceptanceContract =
        serde_json::from_slice(&std::fs::read(tasks.join(ACCEPTANCE_CONTRACT_FILE)).unwrap())
            .unwrap();
    contract.prd.digest = content_digest(text.as_bytes());
    contract.acceptance[0].check = serde_json::from_value(serde_json::json!({"kind":"command","command":"test \"$(sh v*)\" = good","cwd":"repo_root"})).unwrap();
    contract.acceptance[0].covers = vec!["REQ-VALUE-001".into()];
    for (id, covers, command) in [
        ("SUP-REQ-VALUE-002", "REQ-VALUE-002", "test -d ."),
        ("SUP-REQ-VALUE-002-R2", "REQ-VALUE-002", "test -f absent"),
        (
            "SUP-REQ-VALUE-003",
            "REQ-VALUE-003",
            "test 'It keeps the good' = 'It keeps the good'",
        ),
    ] {
        let mut check = contract.acceptance[0].clone();
        check.id = id.into();
        check.covers = vec![covers.into()];
        check.check = serde_json::from_value(
            serde_json::json!({"kind":"command","command":command,"cwd":"repo_root"}),
        )
        .unwrap();
        contract.supplementary.push(check);
    }
    let bytes = serde_json::to_vec_pretty(&contract).unwrap();
    let mut lock: AcceptanceLock =
        serde_json::from_slice(&std::fs::read(tasks.join(ACCEPTANCE_LOCK_FILE)).unwrap()).unwrap();
    lock.digest = content_digest(&bytes);
    lock.baseline_commit = Some(base);
    let mut outcomes = std::collections::BTreeMap::new();
    for check in contract.acceptance.iter().chain(&contract.supplementary) {
        let AcceptanceCheck::Command { command, .. } = &check.check else {
            panic!()
        };
        let status = std::process::Command::new("sh")
            .args(["-c", command])
            .current_dir(temp.path())
            .status()
            .unwrap();
        outcomes.insert(
            check.id.clone(),
            if status.success() {
                BaselineOutcome::Passed
            } else {
                BaselineOutcome::Failed
            },
        );
    }
    lock.baseline_results =
        BaselineResults::observed(lock.baseline_commit.clone(), &outcomes, &contract).outcomes;
    pin.acceptance_digest = lock.digest.clone();
    pin.check_sources_digest = None;
    std::fs::write(tasks.join(ACCEPTANCE_CONTRACT_FILE), bytes).unwrap();
    std::fs::write(
        tasks.join(ACCEPTANCE_LOCK_FILE),
        serde_json::to_vec_pretty(&lock).unwrap(),
    )
    .unwrap();
    std::fs::write(
        acceptance_pin_path(temp.path(), &tasks),
        serde_json::to_vec_pretty(&pin).unwrap(),
    )
    .unwrap();
    archon_workflow::repository_record::write_repository_record(
        &tasks,
        &archon_workflow::repository_record::RepositoryRecordV1 {
            schema_version: 1,
            repository_root: temp
                .path()
                .canonicalize()
                .map(archon_shell::paths::plain)
                .unwrap()
                .display()
                .to_string(),
            base_commit: lock.baseline_commit.clone().unwrap(),
            decomposition_run_id: "fixture".into(),
            recorded_at: "fixture".into(),
        },
    )
    .unwrap();
    let raw = serde_json::json!({"schema_version":1,"acceptance_digest":lock.digest,"tasks":[
        {"task_id":"TASK-X-001","file_name":"TASK-X-001.md","implements":["REQ-VALUE-001"],"deliverable_contracts":[{"kind":"output","artifact_path":"value","min_instances":1,"implements":["REQ-VALUE-001"]}]},
        {"task_id":"TASK-X-002","file_name":"TASK-X-002.md","implements":["REQ-VALUE-002"],"depends_on":[{"task_id":"TASK-X-001","ordering_only":true}],"deliverable_contracts":[{"kind":"output","artifact_path":"other","min_instances":1,"implements":["REQ-VALUE-002"]}]},
        {"task_id":"TASK-X-003","file_name":"TASK-X-003.md","implements":["REQ-VALUE-003"],"deliverable_contracts":[{"kind":"output","artifact_path":"note","artifact_format":"text","min_instances":1,"implements":["REQ-VALUE-003"]}]}
    ]});
    let prepared = prepare_skeleton_freeze_from_candidate(
        temp.path(),
        &tasks,
        &prd,
        GateMode::Observe,
        serde_json::to_vec(&raw).unwrap(),
    )
    .unwrap();
    assert_eq!(
        prepared
            .classification()
            .unwrap()
            .entries
            .iter()
            .filter(|e| e.class
                == archon_workflow::v2::requirement_classes::RequirementClass::CandidateDone)
            .count(),
        3
    );
    let server = server::Server::start();
    let mut config = archon_core::config::ArchonConfig::default();
    config.workflow.gate_mode = GateMode::Observe;
    config.api.base_url = Some(server.url.clone());
    let env = archon_core::env_vars::load_env_vars_from(
        &[("ANTHROPIC_API_KEY".into(), "fixture".into())].into(),
    );
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let resume = crate::command::workflow_freeze_budget::FreezeResume::saving(
        crate::command::workflow_freeze_budget::FreezeBudget::unlimited(),
        true,
    );
    config.workflow.proof.max_runs_per_attempt = 1;
    let stopped = runtime.block_on(
        crate::command::workflow::workflow_freeze_cli::skeleton_proof::prove_and_decide(
            temp.path(),
            &config,
            &env,
            prepared,
            &resume,
        ),
    );
    let error = stopped
        .err()
        .expect("run budget must stop the staged proof");
    let incomplete = error
        .downcast_ref::<crate::command::workflow_freeze_budget::FreezeIncomplete>()
        .expect("typed resumable stop");
    assert!(incomplete.report().contains("proof work remains"));
    assert!(!incomplete.report().contains("made no progress"));
    config.workflow.proof.max_runs_per_attempt = 50;
    let prepared = prepare_skeleton_freeze_from_candidate(
        temp.path(),
        &tasks,
        &prd,
        GateMode::Observe,
        serde_json::to_vec(&raw).unwrap(),
    )
    .unwrap();
    let prepared = runtime
        .block_on(
            crate::command::workflow::workflow_freeze_cli::skeleton_proof::prove_and_decide(
                temp.path(),
                &config,
                &env,
                prepared,
                &resume,
            ),
        )
        .unwrap();
    let findings = prepared.findings.clone();
    let identity = prepared.publication_identity();
    let mut disposition = crate::command::workflow_gate::run_sync_gate(
        temp.path(),
        GateMode::Observe,
        GateId::FreezeSkeleton,
        || {
            Ok(
                crate::command::workflow_gate::GateEvaluation::new("", findings)
                    .with_publication_identity(identity),
            )
        },
    )
    .unwrap();
    publish_skeleton_freeze(prepared, disposition.take_publication_permit().unwrap()).unwrap();
    let calls = server.finish();
    eprintln!("skeleton live corpus provider requests: {calls}");
    let skeleton: TaskSkeleton =
        serde_json::from_slice(&std::fs::read(tasks.join(TASK_SKELETON_FILE)).unwrap()).unwrap();
    for (task, expected) in skeleton.tasks.iter().zip([
        RequirementState::Done,
        RequirementState::Missing,
        RequirementState::Undecided,
    ]) {
        assert_eq!(
            task.requirement_states.values().next().unwrap().state,
            expected,
            "{task:?}"
        );
        let expected_reason = match expected {
            RequirementState::Done => "breakage_kill",
            RequirementState::Missing => "base_failing_check",
            RequirementState::Undecided => "breaker_unusable",
            RequirementState::Inspection => unreachable!(),
        };
        assert_eq!(
            task.requirement_states.values().next().unwrap().reason,
            expected_reason,
            "{task:?}"
        );
        let body = format!(
            "```yaml\ntask_id: {}\ntitle: item\ncomplexity: low\nstatus: ready\ndepends_on: []\nblocks: []\nimplements: []\nrequired_env_keys: []\nrequired_tools: []\ndeliverable_contracts: []\n```\n",
            task.task_id
        );
        let (body, _) =
            crate::command::workflow_host_command_binding::stamp_frozen_frontmatter(&body, task)
                .unwrap();
        std::fs::write(tasks.join(&task.file_name), body).unwrap();
    }
    assert_eq!(skeleton.tasks[0].status.as_deref(), Some("done"));
    assert!(!skeleton.tasks[0].proven_done.is_empty());
    let published_pin: AcceptancePin =
        serde_json::from_slice(&std::fs::read(acceptance_pin_path(temp.path(), &tasks)).unwrap())
            .unwrap();
    assert_eq!(
        archon_workflow::task_skeleton::validate_full_chain(&tasks, &published_pin).unwrap(),
        skeleton
    );
    let status = crate::command::workflow_decompose_status_classes::render(&tasks);
    assert!(
        status.contains("states: done=1 missing=1 undecided=1 inspection=0"),
        "{status}"
    );
    let universe = archon_workflow::task_universe::extract_task_universe_for_generated_run(
        &format!("Implement the decomposed PRD at {}", tasks.display()),
    )
    .unwrap()
    .unwrap();
    let lifecycle = archon_workflow::generated_lifecycle_support::LifecycleContract {
        task_universe: &universe,
        target_repository_root: None,
    };
    let items: Vec<_> = universe.tasks.iter().map(|t| serde_json::json!({"item_id":t.canonical_task_id,"canonical_task_ids":[t.canonical_task_id],"work_type":"implementation","dependency_ids":t.dependency_ids})).collect();
    for completed in [
        std::collections::BTreeSet::new(),
        ["TASK-X-002".into()].into(),
    ] {
        let ready = archon_workflow::generated_lifecycle_support::ready_items_from(
            &lifecycle, &items, &completed,
        );
        assert!(ready.iter().all(|i| i["item_id"] != "TASK-X-001"));
        if completed.is_empty() {
            assert_eq!(
                ready
                    .iter()
                    .map(|i| i["item_id"].as_str().unwrap())
                    .collect::<Vec<_>>(),
                ["TASK-X-002"]
            );
        }
    }
    assert!(
        calls >= 3,
        "breaker, body gut and relevance must reach the local provider"
    );
}

#[test]
fn skeleton_repository_record_absent_is_listed_without_retry_or_provider() {
    let temp = tempfile::tempdir().unwrap();
    let (tasks, _) = seed_frozen_acceptance(&temp);
    let record = archon_workflow::repository_record::repository_record_path(&tasks);
    if record.exists() {
        std::fs::remove_file(record).unwrap();
    }
    let candidate = serde_json::to_vec(&serde_json::json!({"schema_version":1,"acceptance_digest":"draft","tasks":[{"task_id":"TASK-X-001","file_name":"TASK-X-001.md","implements":["AC-X-001"]}]})).unwrap();
    let prepared = prepare_skeleton_freeze_from_candidate(
        temp.path(),
        &tasks,
        &temp.path().join("prds/PRD-X.md"),
        GateMode::Observe,
        candidate,
    )
    .unwrap();
    assert_eq!(
        prepared.classification().unwrap().error.as_deref(),
        Some("repository_record_absent")
    );
    let mut config = archon_core::config::ArchonConfig::default();
    config.workflow.gate_mode = GateMode::Observe;
    config.api.base_url = Some("http://127.0.0.1:1/unreachable".into());
    let env = archon_core::env_vars::load_env_vars_from(&Default::default());
    let resume = crate::command::workflow_freeze_budget::FreezeResume::saving(
        crate::command::workflow_freeze_budget::FreezeBudget::unlimited(),
        true,
    );
    let prepared = tokio::runtime::Runtime::new()
        .unwrap()
        .block_on(
            crate::command::workflow::workflow_freeze_cli::skeleton_proof::prove_and_decide(
                temp.path(),
                &config,
                &env,
                prepared,
                &resume,
            ),
        )
        .unwrap();
    let (_, files) = prepared.into_staged_parts();
    let (_, bytes) = files
        .iter()
        .find(|(name, _)| name == TASK_SKELETON_FILE)
        .unwrap();
    let skeleton: TaskSkeleton = serde_json::from_slice(bytes).unwrap();
    let d = &skeleton.tasks[0].requirement_states["AC-X-001"];
    assert_eq!(d.state, RequirementState::Undecided);
    assert_eq!(d.reason, "classification_unavailable");
    assert_eq!(d.notes, ["repository_record_absent"]);
    assert_eq!(skeleton.tasks[0].status.as_deref(), Some("kept"));
}
