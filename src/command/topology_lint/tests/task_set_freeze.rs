use super::*;

#[test]
fn draft_and_pending_skeletons_accept_constraints_and_numbered_sections() {
    check_draft_and_pending(&[false, true], true);
}

fn check_draft_and_pending(cases: &[bool], check_body: bool) {
    use archon_workflow::task_set_contract::{
        ACCEPTANCE_CONTRACT_FILE, ACCEPTANCE_LOCK_FILE, AcceptanceContract, AcceptanceLock,
        AcceptancePin, TASK_SKELETON_FILE, TASK_SKELETON_LOCK_FILE, content_digest,
    };
    use archon_workflow::task_skeleton::{TaskSkeleton, TaskSkeletonLock};

    for &pending in cases {
        let temp = tempfile::tempdir().unwrap();
        let task = write_task_file_lint_fixture(temp.path());
        let tasks = task.parent().unwrap();
        let prd_path = temp.path().join("prds/PRD-X.md");
        let prd = format!(
            "{}\n- QX-001: restrict work\n- QX-002: restrict other work\n## 7. Widgets\n### 7.1 Small\nRequired small widgets.\n## 8 Empty leaf\n---\n## 9 Non-goals\nExcluded.\n",
            std::fs::read_to_string(&prd_path).unwrap()
        );
        std::fs::write(&prd_path, &prd).unwrap();
        let contract_path = tasks.join(ACCEPTANCE_CONTRACT_FILE);
        let mut contract: AcceptanceContract =
            serde_json::from_slice(&std::fs::read(&contract_path).unwrap()).unwrap();
        contract.prd.digest = content_digest(prd.as_bytes());
        let bytes = serde_json::to_vec_pretty(&contract).unwrap();
        std::fs::write(&contract_path, &bytes).unwrap();
        let digest = content_digest(&bytes);
        let lock_path = tasks.join(ACCEPTANCE_LOCK_FILE);
        let mut lock: AcceptanceLock =
            serde_json::from_slice(&std::fs::read(&lock_path).unwrap()).unwrap();
        lock.digest = digest.clone();
        std::fs::write(&lock_path, serde_json::to_vec_pretty(&lock).unwrap()).unwrap();
        let pin_path = crate::command::workflow_task_set::acceptance_pin_path(temp.path(), tasks);
        let mut pin: AcceptancePin =
            serde_json::from_slice(&std::fs::read(&pin_path).unwrap()).unwrap();
        pin.acceptance_digest = digest.clone();
        pin.skeleton_digest = None;
        pin.skeleton_gate = None;
        std::fs::write(&pin_path, serde_json::to_vec_pretty(&pin).unwrap()).unwrap();
        let skeleton_path = tasks.join(TASK_SKELETON_FILE);
        let mut skeleton: TaskSkeleton =
            serde_json::from_slice(&std::fs::read(&skeleton_path).unwrap()).unwrap();
        skeleton.tasks[0].source_sections = vec!["0".into(), "7.1".into(), "8".into()];
        skeleton.tasks[0]
            .implements
            .extend(["QX-001".into(), "QX-404".into()]);
        if !pending {
            skeleton.acceptance_digest = digest;
        }
        let bytes = serde_json::to_vec_pretty(&skeleton).unwrap();
        std::fs::write(&skeleton_path, &bytes).unwrap();
        let lock_path = tasks.join(TASK_SKELETON_LOCK_FILE);
        if pending {
            let mut lock: TaskSkeletonLock =
                serde_json::from_slice(&std::fs::read(&lock_path).unwrap()).unwrap();
            lock.digest = content_digest(&bytes);
            std::fs::write(&lock_path, serde_json::to_vec_pretty(&lock).unwrap()).unwrap();
        } else {
            std::fs::remove_file(&lock_path).unwrap();
        }
        if pending && check_body {
            let lint =
                task_file::inspect(temp.path(), &task, archon_core::config::GateMode::Enforce);
            assert!(lint.report.contains("unfrozen draft"));
            let gate = evaluate_lint(
                temp.path(),
                &LintSource::TaskFile(task.clone()),
                archon_core::config::GateMode::Enforce,
            )
            .unwrap();
            let candidate = crate::command::topology_lint::evaluate_task_file_candidate(
                temp.path(),
                &task,
                &std::fs::read(&task).unwrap(),
                archon_core::config::GateMode::Enforce,
            )
            .unwrap();
            for finding in &candidate.findings {
                if finding.deterministic_defect.as_ref().is_some_and(|d| {
                    matches!(
                        d.code.as_str(),
                        "uncited_section" | "unknown_section" | "unowned_constraint"
                    )
                }) {
                    assert_eq!(
                        finding.remediation_scope,
                        archon_workflow::RemediationScope::Skeleton
                    );
                    assert_eq!(
                        finding.source_path.as_deref(),
                        Some(skeleton_path.as_path())
                    );
                }
            }
            assert!(candidate.findings.iter().any(|f| {
                f.deterministic_defect
                    .as_ref()
                    .is_some_and(|d| d.code == "unowned_constraint")
            }));
            let finding = gate
                .findings
                .iter()
                .find(|f| {
                    f.deterministic_defect
                        .as_ref()
                        .is_some_and(|d| d.code == "unowned_constraint")
                })
                .unwrap();
            assert_eq!(
                finding.remediation_scope,
                archon_workflow::RemediationScope::Skeleton
            );
            assert_eq!(
                finding.source_path.as_deref(),
                Some(skeleton_path.as_path())
            );
            assert_eq!(
                finding.text,
                archon_workflow::task_skeleton::constraint_owner_findings(
                    &skeleton,
                    &archon_workflow::obligation_ids::constraint_texts(&prd)
                )[0]
                .message
            );
            assert!(lint.deterministic.values().flatten().any(|defect| defect.code == "unowned_constraint" && defect.subject == "QX-002"));
            let defects: Vec<_> = lint
                .deterministic
                .values()
                .flatten()
                .filter(|defect| {
                    matches!(
                        defect.code.as_str(),
                        "unknown_obligation" | "unowned_obligation"
                    )
                })
                .collect();
            assert_eq!(defects.len(), 1, "{:?}", lint.blockers);
            assert_eq!(defects[0].code, "unknown_obligation");
            assert!(!lint.blockers.iter().any(|text| text.contains("QX-001")));
            let mut entries = skeleton.clone();
            entries.tasks[0]
                .source_sections
                .extend(["7.1".into(), " 8".into()]);
            let entry_bytes = serde_json::to_vec_pretty(&entries).unwrap();
            std::fs::write(&skeleton_path, &entry_bytes).unwrap();
            let mut entry_lock: TaskSkeletonLock =
                serde_json::from_slice(&std::fs::read(&lock_path).unwrap()).unwrap();
            entry_lock.digest = content_digest(&entry_bytes);
            std::fs::write(&lock_path, serde_json::to_vec_pretty(&entry_lock).unwrap()).unwrap();
            let gate = evaluate_lint(
                temp.path(),
                &LintSource::TaskFile(task.clone()),
                archon_core::config::GateMode::Enforce,
            )
            .unwrap();
            for code in ["duplicate_entry", "untrimmed_entry"] {
                let finding = gate
                    .findings
                    .iter()
                    .find(|f| {
                        f.deterministic_defect
                            .as_ref()
                            .is_some_and(|d| d.code == code)
                    })
                    .unwrap();
                assert_eq!(
                    finding.remediation_scope,
                    archon_workflow::RemediationScope::Skeleton
                );
                assert_eq!(
                    finding.source_path.as_deref(),
                    Some(skeleton_path.as_path())
                );
            }
            std::fs::write(&skeleton_path, &bytes).unwrap();
            entry_lock.digest = content_digest(&bytes);
            std::fs::write(&lock_path, serde_json::to_vec_pretty(&entry_lock).unwrap()).unwrap();
        }
        let lint =
            task_set::inspect(temp.path(), tasks, archon_core::config::GateMode::Enforce).unwrap();
        assert!(lint.report.contains(if pending {
            "unfrozen draft"
        } else {
            "draft skeleton"
        }));
        let defects: Vec<_> = lint.deterministic.values().flatten().collect();
        assert_eq!(defects.len(), 2, "{:?}", lint.blockers);
        assert!(defects.iter().any(|defect| defect.code == "unknown_obligation" && defect.subject == "TASK-X-010"));
        assert!(
            defects
                .iter()
                .any(|defect| defect.code == "unowned_constraint" && defect.subject == "QX-002")
        );
        assert!(lint.blockers.iter().any(|text| text.contains("QX-404")));
        assert!(!lint.blockers.iter().any(|text| text.contains("QX-001")));
        // Exercise the complete gate: section coverage must not also be
        // appended by task_set::inspect for this same draft/pending skeleton.
        skeleton.tasks[0].source_sections = vec!["0".into(), "7.1".into(), "404".into()];
        let bytes = serde_json::to_vec_pretty(&skeleton).unwrap();
        std::fs::write(&skeleton_path, &bytes).unwrap();
        if pending {
            let mut lock: TaskSkeletonLock =
                serde_json::from_slice(&std::fs::read(&lock_path).unwrap()).unwrap();
            lock.digest = content_digest(&bytes);
            std::fs::write(&lock_path, serde_json::to_vec_pretty(&lock).unwrap()).unwrap();
        }
        let evaluation = evaluate_lint(
            temp.path(),
            &LintSource::Tasks(tasks.to_path_buf()),
            archon_core::config::GateMode::Enforce,
        )
        .unwrap();
        for code in ["uncited_section", "unknown_section", "unowned_constraint"] {
            let findings: Vec<_> = evaluation
                .findings
                .iter()
                .filter(|finding| {
                    finding
                        .deterministic_defect
                        .as_ref()
                        .is_some_and(|defect| defect.code == code)
                })
                .collect();
            assert_eq!(findings.len(), 1, "{code}: {:?}", evaluation.findings);
        }
    }
}

#[test]
fn directory_lint_renders_kind_mismatch_as_information_only() {
    use archon_workflow::task_set_contract::{TASK_SKELETON_FILE, TASK_SKELETON_LOCK_FILE};

    let temp = tempfile::tempdir().unwrap();
    let task_path = write_task_file_lint_fixture(temp.path());
    let tasks = task_path.parent().unwrap();
    let skeleton_path = tasks.join(TASK_SKELETON_FILE);
    let mut skeleton: archon_workflow::task_skeleton::TaskSkeleton =
        serde_json::from_slice(&std::fs::read(&skeleton_path).unwrap()).unwrap();
    skeleton.tasks.insert(
        0,
        archon_workflow::task_skeleton::FrozenTask {
            status: None,
            task_id: "TASK-X-001".into(),
            file_name: "TASK-X-001-base.md".into(),
            depends_on: Vec::new(),
            blocks: Vec::new(),
            implements: Vec::new(),
            source_sections: Vec::new(),
            deliverable_contracts: vec![
                archon_workflow::task_universe::WorkflowV2DeliverableContract {
                    kind: "producer_label".into(),
                    artifact_path: "out.json".into(),
                    typed_verifier_command: Some("grep -q required {artifact_path}".into()),
                    ..Default::default()
                },
            ],
        },
    );
    skeleton.tasks[1].depends_on = vec![archon_workflow::task_skeleton::FrozenDependency {
        task_id: "TASK-X-001".into(),
        consumes: vec![archon_workflow::task_skeleton::ConsumedArtifact {
            artifact_path: "out.json".into(),
            kind: Some("consumer_label".into()),
            ..Default::default()
        }],
        ordering_only: false,
    }];
    let bytes = serde_json::to_vec_pretty(&skeleton).unwrap();
    std::fs::write(&skeleton_path, &bytes).unwrap();
    let digest = archon_workflow::task_set_contract::content_digest(&bytes);
    std::fs::write(
        tasks.join(TASK_SKELETON_LOCK_FILE),
        serde_json::to_vec_pretty(&archon_workflow::task_skeleton::TaskSkeletonLock {
            algorithm: "blake3".into(),
            digest: digest.clone(),
            acceptance_digest: skeleton.acceptance_digest.clone(),
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

    let report = run_lint(temp.path(), &LintSource::Tasks(tasks.to_path_buf())).unwrap();
    assert!(
        report.contains("dependency contract information (non-blocking)"),
        "{report}"
    );
    assert!(report.contains("informational kind mismatch"), "{report}");
    let blockers = blocking_findings(temp.path(), &LintSource::Tasks(tasks.to_path_buf()));
    assert!(
        !blockers
            .iter()
            .any(|finding| finding.contains("kind mismatch")),
        "{blockers:?}"
    );
}

#[test]
fn off_mode_skips_lint_input_and_observe_records_real_policy_findings() {
    let off = tempfile::tempdir().unwrap();
    let source = LintSource::TaskFile(off.path().join("does-not-exist.md"));
    let disposition = crate::command::workflow_gate::run_sync_gate(
        off.path(),
        archon_core::config::GateMode::Off,
        crate::command::workflow_gate::GateId::WorkflowLintTaskFile,
        || evaluate_lint(off.path(), &source, archon_core::config::GateMode::Off),
    )
    .unwrap();
    assert_eq!(
        disposition.report(),
        crate::command::workflow_gate::OFF_MESSAGE
    );
    assert!(!off.path().join(".archon").exists());

    let observed = tempfile::tempdir().unwrap();
    let task = write_task_file_lint_fixture(observed.path());
    let raw = std::fs::read_to_string(&task).unwrap();
    std::fs::write(&task, raw.replace("status: ready", "status: blocked")).unwrap();
    let source = LintSource::TaskFile(task);
    let disposition = crate::command::workflow_gate::run_sync_gate(
        observed.path(),
        archon_core::config::GateMode::Observe,
        crate::command::workflow_gate::GateId::WorkflowLintTaskFile,
        || {
            evaluate_lint(
                observed.path(),
                &source,
                archon_core::config::GateMode::Observe,
            )
        },
    )
    .unwrap();
    assert!(!disposition.is_blocked());
    assert!(
        disposition
            .diagnostics()
            .iter()
            .any(|line| line.contains("status: blocked"))
    );
    assert!(crate::command::workflow_gate::shadow_log_path(observed.path()).is_file());
}

#[test]
fn enforce_lint_refuses_predecessor_freeze_with_observed_findings() {
    let temp = tempfile::tempdir().unwrap();
    let task = write_task_file_lint_fixture(temp.path());
    let tasks = task.parent().unwrap();
    let pin_path = crate::command::workflow_task_set::acceptance_pin_path(temp.path(), tasks);
    let mut pin: archon_workflow::task_set_contract::AcceptancePin =
        serde_json::from_slice(&std::fs::read(&pin_path).unwrap()).unwrap();
    pin.acceptance_gate.mode = archon_workflow::task_set_contract::FreezeGateMode::Observe;
    pin.acceptance_gate.finding_count = 1;
    pin.acceptance_gate.findings_digest =
        archon_workflow::task_set_contract::content_digest(b"observed finding");
    let lock_path = tasks.join(archon_workflow::task_set_contract::ACCEPTANCE_LOCK_FILE);
    let mut lock: archon_workflow::task_set_contract::AcceptanceLock =
        serde_json::from_slice(&std::fs::read(&lock_path).unwrap()).unwrap();
    lock.gate = pin.acceptance_gate.clone();
    std::fs::write(&pin_path, serde_json::to_vec_pretty(&pin).unwrap()).unwrap();
    std::fs::write(&lock_path, serde_json::to_vec_pretty(&lock).unwrap()).unwrap();

    let evaluation = evaluate_lint(
        temp.path(),
        &LintSource::TaskFile(task),
        archon_core::config::GateMode::Enforce,
    )
    .unwrap();
    assert!(
        evaluation.findings.iter().any(|finding| {
            finding.text.contains("predecessor acceptance freeze")
                && finding.text.contains("re-freeze under enforce")
        }),
        "{:?}",
        evaluation.findings
    );
}

#[test]
fn partial_acceptance_freeze_is_operational_even_in_observe_mode() {
    use archon_workflow::task_set_contract::{
        ACCEPTANCE_LOCK_FILE, TASK_SKELETON_FILE, TASK_SKELETON_LOCK_FILE,
    };

    let temp = tempfile::tempdir().unwrap();
    let task = write_task_file_lint_fixture(temp.path());
    let tasks = task.parent().unwrap();
    let pin_path = crate::command::workflow_task_set::acceptance_pin_path(temp.path(), tasks);
    std::fs::remove_file(pin_path).unwrap();
    std::fs::remove_file(tasks.join(ACCEPTANCE_LOCK_FILE)).unwrap();
    std::fs::remove_file(tasks.join(TASK_SKELETON_FILE)).unwrap();
    std::fs::remove_file(tasks.join(TASK_SKELETON_LOCK_FILE)).unwrap();

    let source = LintSource::Tasks(tasks.to_path_buf());
    let error = crate::command::workflow_gate::run_sync_gate(
        temp.path(),
        archon_core::config::GateMode::Observe,
        crate::command::workflow_gate::GateId::WorkflowLintTaskSet,
        || evaluate_lint(temp.path(), &source, archon_core::config::GateMode::Observe),
    )
    .unwrap_err()
    .to_string();
    assert!(error.contains("partial acceptance freeze"), "{error}");
    assert!(!crate::command::workflow_gate::shadow_log_path(temp.path()).exists());
}

#[test]
fn graph_lowering_failure_preserves_report_but_is_operational_in_observe() {
    let temp = tempfile::tempdir().unwrap();
    let tasks = temp.path().join("tasks/PRD-X");
    std::fs::create_dir_all(&tasks).unwrap();
    for (task_id, dependency) in [("TASK-X-010", "TASK-X-020"), ("TASK-X-020", "TASK-X-010")] {
        std::fs::write(
            tasks.join(format!("{task_id}-body.md")),
            format!(
                "```yaml\ntask_id: {task_id}\ntitle: Body\ncomplexity: medium\nstatus: ready\ndepends_on: [{dependency}]\nblocks: []\nimplements: []\nrequired_env_keys: []\nrequired_tools: [sh]\ndeliverable_contracts: []\n```\n\n# Body\n\n## Focused Tests\n- `sh -c 'exit 1'`\n"
            ),
        )
        .unwrap();
    }
    let source = LintSource::Tasks(tasks);
    let disposition = crate::command::workflow_gate::run_sync_gate(
        temp.path(),
        archon_core::config::GateMode::Observe,
        crate::command::workflow_gate::GateId::WorkflowLintTaskSet,
        || evaluate_lint(temp.path(), &source, archon_core::config::GateMode::Observe),
    )
    .unwrap();

    assert!(disposition.report().contains("NOT ANALYSED"));
    let error = disposition.require_allowed().unwrap_err().to_string();
    assert!(error.contains("dependency cycle"), "{error}");
    assert!(!crate::command::workflow_gate::shadow_log_path(temp.path()).exists());
}

#[test]
fn directory_lint_rejects_stale_prd_content_even_when_ids_are_unchanged() {
    let temp = tempfile::tempdir().unwrap();
    let task = write_task_file_lint_fixture(temp.path());
    let tasks = task.parent().unwrap().to_path_buf();
    let prd = temp.path().join("prds/PRD-X.md");
    let raw = std::fs::read_to_string(&prd).unwrap();
    std::fs::write(
        &prd,
        raw.replace("output is valid", "output is valid after mutation"),
    )
    .unwrap();

    let error = evaluate_lint(
        temp.path(),
        &LintSource::Tasks(tasks),
        archon_core::config::GateMode::Observe,
    )
    .unwrap_err()
    .to_string();
    assert!(error.contains("PRD digest mismatch"), "{error}");
    assert!(
        error.contains("re-run workflow freeze-acceptance"),
        "{error}"
    );
}

#[test]
#[allow(non_snake_case)]
fn P5_3_pending_body_landing_constraint_ownership() {
    check_draft_and_pending(&[true], true);
}
#[test]
#[allow(non_snake_case)]
fn P5_4_draft_pending_constraint_identity_deduplicates() {
    check_draft_and_pending(&[false, true], false);
}
