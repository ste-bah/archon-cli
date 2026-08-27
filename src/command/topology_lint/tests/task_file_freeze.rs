use super::*;

fn clean_stamp() -> archon_workflow::task_set_contract::FreezeGateStamp {
    archon_workflow::task_set_contract::FreezeGateStamp {
        mode: archon_workflow::task_set_contract::FreezeGateMode::Enforce,
        finding_count: 0,
        findings_digest: archon_workflow::task_set_contract::empty_gate_findings_digest(),
        binary_commit: "test-revision".into(),
        evaluated_at: "2026-08-26T18:30:00Z".into(),
    }
}

fn write_task_file_lint_fixture(root: &Path) -> std::path::PathBuf {
    use archon_workflow::task_set_contract::{
        ACCEPTANCE_CONTRACT_FILE, ACCEPTANCE_LOCK_FILE, AcceptanceLock, AcceptancePin,
        TASK_SKELETON_FILE, TASK_SKELETON_LOCK_FILE, content_digest,
    };
    use archon_workflow::task_skeleton::{FrozenTask, TaskSkeleton, TaskSkeletonLock};

    let tasks = root.join("tasks/PRD-X");
    std::fs::create_dir_all(&tasks).unwrap();
    let prd = root.join("prds/PRD-X.md");
    std::fs::create_dir_all(prd.parent().unwrap()).unwrap();
    let prd_bytes =
        b"## Acceptance Criteria\n| ID | Criterion |\n|---|---|\n| AC-X-001 | output is valid |\n";
    std::fs::write(&prd, prd_bytes).unwrap();
    let acceptance = format!(
        r#"{{
          "schema_version":1,
          "prd":{{"path":"prds/PRD-X.md","digest":"{}"}},
          "gap_policy":{{"permitted_acceptance_ids":[],"forbidden_phrases":[],"required_fields":[]}},
          "acceptance":[{{"id":"AC-X-001","criterion":"output is valid","check":{{"kind":"command","command":"sh -c 'exit 0'","cwd":"project_root"}},"gap_permitted":false,"judgment":{{"verdict":"accepted","counterexample":"attempted","reason":"rejects","host_call_id":"judge-1"}}}}],
          "supplementary":[]
        }}"#,
        content_digest(prd_bytes)
    );
    std::fs::write(tasks.join(ACCEPTANCE_CONTRACT_FILE), acceptance.as_bytes()).unwrap();
    let acceptance_digest = content_digest(acceptance.as_bytes());
    std::fs::write(
        tasks.join(ACCEPTANCE_LOCK_FILE),
        serde_json::to_vec_pretty(&AcceptanceLock {
            algorithm: "blake3".into(),
            digest: acceptance_digest.clone(),
            gate: clean_stamp(),
        })
        .unwrap(),
    )
    .unwrap();
    let task_path = tasks.join("TASK-X-010-body.md");
    std::fs::write(
        &task_path,
        "# Body\n\n```yaml\ntask_id: TASK-X-010\ntitle: Body\ncomplexity: medium\nstatus: ready\ndepends_on: []\nblocks: []\nimplements: [AC-X-001]\nrequired_env_keys: []\nrequired_tools: [sh]\ndeliverable_contracts: []\n```\n\n## Acceptance Criteria\n- Output is valid.\n\n## Focused Tests\n- `sh -c 'grep -q Body TASK-X-010-body.md'`\n",
    )
    .unwrap();
    let skeleton = TaskSkeleton {
        schema_version: 1,
        acceptance_digest: acceptance_digest.clone(),
        tasks: vec![FrozenTask {
            task_id: "TASK-X-010".into(),
            file_name: "TASK-X-010-body.md".into(),
            depends_on: Vec::new(),
            blocks: Vec::new(),
            implements: vec!["AC-X-001".into()],
            deliverable_contracts: Vec::new(),
        }],
    };
    let skeleton_bytes = serde_json::to_vec_pretty(&skeleton).unwrap();
    std::fs::write(tasks.join(TASK_SKELETON_FILE), &skeleton_bytes).unwrap();
    let skeleton_digest = content_digest(&skeleton_bytes);
    std::fs::write(
        tasks.join(TASK_SKELETON_LOCK_FILE),
        serde_json::to_vec_pretty(&TaskSkeletonLock {
            algorithm: "blake3".into(),
            digest: skeleton_digest.clone(),
            acceptance_digest: acceptance_digest.clone(),
            gate: clean_stamp(),
        })
        .unwrap(),
    )
    .unwrap();
    let pin = AcceptancePin {
        task_root: tasks.canonicalize().unwrap().display().to_string(),
        acceptance_digest,
        freeze_event_id: "freeze-1".into(),
        acceptance_gate: clean_stamp(),
        skeleton_digest: Some(skeleton_digest),
        skeleton_gate: Some(clean_stamp()),
    };
    let pin_path = crate::command::workflow_task_set::acceptance_pin_path(root, &tasks);
    std::fs::create_dir_all(pin_path.parent().unwrap()).unwrap();
    std::fs::write(pin_path, serde_json::to_vec_pretty(&pin).unwrap()).unwrap();
    task_path
}

#[test]
fn task_file_lint_uses_runtime_parser_and_frozen_structural_equality() {
    let temp = tempfile::tempdir().unwrap();
    let task = write_task_file_lint_fixture(temp.path());
    let source = LintSource::TaskFile(task.clone());
    let report = run_lint(temp.path(), &source).unwrap();
    assert!(
        report.contains("parsed with runtime parse_task_file"),
        "{report}"
    );
    assert!(report.contains("frozen fields match"), "{report}");
    assert!(
        report.contains("coverage: NOT ANALYSED for --task-file"),
        "{report}"
    );
    assert!(
        report.contains("edges: NOT ANALYSED for --task-file"),
        "{report}"
    );
    assert!(
        blocking_findings(temp.path(), &source).is_empty(),
        "{report}"
    );

    let raw = std::fs::read_to_string(&task).unwrap();
    std::fs::write(
        &task,
        raw.replace("implements: [AC-X-001]", "implements: []"),
    )
    .unwrap();
    let findings = blocking_findings(temp.path(), &source);
    assert!(
        findings.iter().any(|finding| {
            finding.contains("implements") && finding.contains("restore the frozen value")
        }),
        "{findings:?}"
    );
    assert!(
        findings.iter().any(|finding| {
            finding.contains("AC-X-001") && finding.contains("claimed by no task")
        }),
        "{findings:?}"
    );
}

#[test]
fn task_file_is_mutually_exclusive_with_every_other_lint_source() {
    let task = Path::new("TASK-X-001.md");
    for result in [
        LintSource::from_flags(Some(task), Some(Path::new("tasks")), None, None),
        LintSource::from_flags(Some(task), None, Some(Path::new("spec.yml")), None),
        LintSource::from_flags(Some(task), None, None, Some("graph")),
    ] {
        assert!(result.unwrap_err().to_string().contains("exactly one"));
    }
}

#[test]
fn directory_lint_blocks_when_a_task_diverges_from_the_frozen_skeleton() {
    let temp = tempfile::tempdir().unwrap();
    let task = write_task_file_lint_fixture(temp.path());
    let tasks = task.parent().unwrap().to_path_buf();
    let source = LintSource::Tasks(tasks.clone());
    assert!(
        !blocking_findings(temp.path(), &source)
            .iter()
            .any(|finding| finding.contains("frozen")),
        "matching frozen set must not produce a skeleton finding"
    );
    let raw = std::fs::read_to_string(&task).unwrap();
    std::fs::write(
        &task,
        raw.replace("implements: [AC-X-001]", "implements: []"),
    )
    .unwrap();
    let findings = blocking_findings(temp.path(), &source);
    assert!(
        findings
            .iter()
            .any(|finding| finding.contains("implements") && finding.contains("restore")),
        "{findings:?}"
    );
}

#[test]
fn task_file_lint_uses_runtime_status_rules() {
    let temp = tempfile::tempdir().unwrap();
    let task = write_task_file_lint_fixture(temp.path());
    let raw = std::fs::read_to_string(&task).unwrap();
    std::fs::write(&task, raw.replace("status: ready", "status: blocked")).unwrap();
    let findings = blocking_findings(temp.path(), &LintSource::TaskFile(task));
    assert!(
        findings.iter().any(|finding| {
            finding.contains("status: blocked")
                && finding.contains("no dependency")
                && finding.contains("dependency is missing or the status is stale")
        }),
        "{findings:?}"
    );
}

#[test]
fn legacy_directory_without_freezes_is_not_reported_as_frozen_or_blocked_on_absence() {
    let temp = tempfile::tempdir().unwrap();
    let tasks = temp.path().join("tasks/PRD-X");
    std::fs::create_dir_all(&tasks).unwrap();
    std::fs::write(
        tasks.join("TASK-X-010-body.md"),
        "# Body\n\n```yaml\ntask_id: TASK-X-010\ntitle: Body\ncomplexity: medium\nstatus: ready\ndepends_on: []\nblocks: []\nimplements: []\nrequired_env_keys: []\nrequired_tools: [sh]\ndeliverable_contracts: []\n```\n\n## Focused Tests\n- `sh -c 'grep -q Body TASK-X-010-body.md'`\n",
    )
    .unwrap();
    let source = LintSource::Tasks(tasks);
    let report = run_lint(temp.path(), &source).unwrap();
    assert!(report.contains("legacy compatibility"), "{report}");
    assert!(report.contains("this is not a freeze pass"), "{report}");
    let findings = blocking_findings(temp.path(), &source);
    assert!(
        !findings.iter().any(|finding| {
            finding.contains("task-set pin") || finding.contains("freeze-acceptance")
        }),
        "{findings:?}"
    );
}

#[test]
fn directory_lint_accepts_the_complete_acceptance_only_phase() {
    use archon_workflow::task_set_contract::{TASK_SKELETON_FILE, TASK_SKELETON_LOCK_FILE};

    let temp = tempfile::tempdir().unwrap();
    let task = write_task_file_lint_fixture(temp.path());
    let tasks = task.parent().unwrap().to_path_buf();
    std::fs::remove_file(tasks.join(TASK_SKELETON_FILE)).unwrap();
    std::fs::remove_file(tasks.join(TASK_SKELETON_LOCK_FILE)).unwrap();
    let pin_path = crate::command::workflow_task_set::acceptance_pin_path(temp.path(), &tasks);
    let mut pin: archon_workflow::task_set_contract::AcceptancePin =
        serde_json::from_slice(&std::fs::read(&pin_path).unwrap()).unwrap();
    pin.skeleton_digest = None;
    pin.skeleton_gate = None;
    std::fs::write(&pin_path, serde_json::to_vec_pretty(&pin).unwrap()).unwrap();

    let evaluation = evaluate_lint(
        temp.path(),
        &LintSource::Tasks(tasks),
        archon_core::config::GateMode::Enforce,
    )
    .unwrap();
    assert!(
        evaluation.report.contains("acceptance freeze is complete"),
        "{}",
        evaluation.report
    );
    assert!(
        evaluation.report.contains("skeleton has not been authored"),
        "{}",
        evaluation.report
    );
    assert!(evaluation.findings.is_empty(), "{:?}", evaluation.findings);
}

#[test]
fn directory_lint_checks_a_draft_skeleton_without_calling_it_frozen() {
    use archon_workflow::task_set_contract::TASK_SKELETON_LOCK_FILE;

    let temp = tempfile::tempdir().unwrap();
    let task = write_task_file_lint_fixture(temp.path());
    let tasks = task.parent().unwrap().to_path_buf();
    std::fs::remove_file(tasks.join(TASK_SKELETON_LOCK_FILE)).unwrap();
    let pin_path = crate::command::workflow_task_set::acceptance_pin_path(temp.path(), &tasks);
    let mut pin: archon_workflow::task_set_contract::AcceptancePin =
        serde_json::from_slice(&std::fs::read(&pin_path).unwrap()).unwrap();
    pin.skeleton_digest = None;
    pin.skeleton_gate = None;
    std::fs::write(&pin_path, serde_json::to_vec_pretty(&pin).unwrap()).unwrap();

    let evaluation = evaluate_lint(
        temp.path(),
        &LintSource::Tasks(tasks),
        archon_core::config::GateMode::Enforce,
    )
    .unwrap();
    assert!(
        evaluation.report.contains("draft skeleton"),
        "{}",
        evaluation.report
    );
    assert!(
        evaluation.report.contains("NOT frozen"),
        "{}",
        evaluation.report
    );
    assert!(evaluation.findings.is_empty(), "{:?}", evaluation.findings);
}

#[test]
fn partial_skeleton_lock_without_a_draft_is_operational() {
    use archon_workflow::task_set_contract::TASK_SKELETON_FILE;

    let temp = tempfile::tempdir().unwrap();
    let task = write_task_file_lint_fixture(temp.path());
    let tasks = task.parent().unwrap().to_path_buf();
    std::fs::remove_file(tasks.join(TASK_SKELETON_FILE)).unwrap();

    let error = evaluate_lint(
        temp.path(),
        &LintSource::Tasks(tasks),
        archon_core::config::GateMode::Observe,
    )
    .unwrap_err()
    .to_string();
    assert!(error.contains("partial skeleton freeze"), "{error}");
}

#[test]
fn unfrozen_task_sets_still_run_structured_edge_policy() {
    let temp = tempfile::tempdir().unwrap();
    let tasks = temp.path().join("tasks/PRD-X");
    std::fs::create_dir_all(&tasks).unwrap();
    for (task_id, depends_on) in [("TASK-X-001", "[]"), ("TASK-X-010", "[TASK-X-001]")] {
        std::fs::write(
            tasks.join(format!("{task_id}-body.md")),
            format!(
                "# Body\n\n```yaml\ntask_id: {task_id}\ntitle: Body\ncomplexity: medium\nstatus: ready\ndepends_on: {depends_on}\nblocks: []\nimplements: []\nrequired_env_keys: []\nrequired_tools: [sh]\ndeliverable_contracts: []\n```\n\n## Focused Tests\n- `sh -c 'exit 1'`\n"
            ),
        )
        .unwrap();
    }

    let evaluation = evaluate_lint(
        temp.path(),
        &LintSource::Tasks(tasks),
        archon_core::config::GateMode::Enforce,
    )
    .unwrap();
    assert!(evaluation.report.contains("legacy compatibility"));
    assert!(
        evaluation.findings.iter().any(|finding| {
            finding.text.contains("TASK-X-010")
                && finding.text.contains("TASK-X-001")
                && finding
                    .text
                    .contains("add a non-empty consumes list or set ordering_only: true")
        }),
        "{:?}",
        evaluation.findings
    );
}

#[test]
fn task_file_lint_blocks_the_same_weak_verifier_as_the_runtime_gate() {
    let temp = tempfile::tempdir().unwrap();
    let task = write_task_file_lint_fixture(temp.path());
    let raw = std::fs::read_to_string(&task).unwrap();
    let raw = raw.replace(
        "deliverable_contracts: []",
        "deliverable_contracts:\n  - kind: report\n    artifact_path: out.json\n    typed_verifier_command: test -f out.json",
    );
    std::fs::write(&task, raw).unwrap();

    let tasks = task.parent().unwrap();
    let skeleton_path = tasks.join(archon_workflow::task_set_contract::TASK_SKELETON_FILE);
    let mut skeleton: archon_workflow::task_skeleton::TaskSkeleton =
        serde_json::from_slice(&std::fs::read(&skeleton_path).unwrap()).unwrap();
    let parsed = archon_workflow::task_universe::parsing::parse_task_file(
        &task,
        &std::fs::read_to_string(&task).unwrap(),
    )
    .unwrap();
    skeleton.tasks[0].deliverable_contracts = parsed.deliverable_contracts.clone();
    let bytes = serde_json::to_vec_pretty(&skeleton).unwrap();
    std::fs::write(&skeleton_path, &bytes).unwrap();
    let digest = archon_workflow::task_set_contract::content_digest(&bytes);
    let acceptance_digest = skeleton.acceptance_digest.clone();
    std::fs::write(
        tasks.join(archon_workflow::task_set_contract::TASK_SKELETON_LOCK_FILE),
        serde_json::to_vec_pretty(&archon_workflow::task_skeleton::TaskSkeletonLock {
            algorithm: "blake3".into(),
            digest: digest.clone(),
            acceptance_digest,
            gate: clean_stamp(),
        })
        .unwrap(),
    )
    .unwrap();
    let pin_path = crate::command::workflow_task_set::acceptance_pin_path(temp.path(), tasks);
    let mut pin: archon_workflow::task_set_contract::AcceptancePin =
        serde_json::from_slice(&std::fs::read(&pin_path).unwrap()).unwrap();
    pin.skeleton_digest = Some(digest);
    pin.skeleton_gate = Some(clean_stamp());
    std::fs::write(pin_path, serde_json::to_vec_pretty(&pin).unwrap()).unwrap();

    let findings = blocking_findings(temp.path(), &LintSource::TaskFile(task));
    assert!(
        findings.iter().any(|finding| {
            finding.contains("only tests whether its own artifact_path")
                && finding.contains("replace")
                && finding.contains("deleting the verifier does not satisfy this contract")
        }),
        "{findings:?}"
    );
}

#[path = "task_file_lifecycle.rs"]
mod task_file_lifecycle;

#[path = "task_set_freeze.rs"]
mod task_set_freeze;

#[test]
fn phase_local_candidate_lint_reads_candidate_bytes_and_preserves_live_task() {
    let temp = tempfile::tempdir().unwrap();
    let task = write_task_file_lint_fixture(temp.path());
    let live_before = std::fs::read(&task).unwrap();
    let candidate = String::from_utf8(live_before.clone()).unwrap().replace(
        "- `sh -c 'grep -q Body TASK-X-010-body.md'`",
        "- Verification remains to be made runnable.",
    );

    let evaluation = evaluate_task_file_candidate(
        temp.path(),
        &task,
        candidate.as_bytes(),
        archon_core::config::GateMode::Observe,
    )
    .unwrap();
    let envelope = evaluation.into_envelope().unwrap();

    assert_eq!(std::fs::read(&task).unwrap(), live_before);
    assert!(
        envelope
            .policy_findings
            .iter()
            .any(|finding| finding.text.contains("runnable focused test")),
        "{:?}",
        envelope.policy_findings
    );
    assert!(
        envelope.policy_findings.iter().all(|finding| {
            finding.remediation_scope == archon_workflow::RemediationScope::Body
        })
    );
    assert!(
        envelope
            .report
            .as_str()
            .is_some_and(|report| report.contains("coverage: NOT ANALYSED for --task-file"))
    );
}
