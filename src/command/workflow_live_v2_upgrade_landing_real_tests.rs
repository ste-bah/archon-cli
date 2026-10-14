use super::*;

struct RealLandingProcess {
    executed: Arc<Mutex<Vec<String>>>,
    staged_task_reads: Arc<Mutex<Vec<(std::path::PathBuf, Vec<u8>)>>>,
    refuse_first_two: bool,
    mutate_frozen_task: bool,
    mutate_destination_on_execute: Option<std::path::PathBuf>,
}

#[async_trait::async_trait]
impl crate::command::workflow_host_command_exec::HostCommandProcessAdapter for RealLandingProcess {
    async fn execute(
        &self,
        request: crate::command::workflow_host_command_catalog::ResolvedHostCommand,
        _control: crate::command::workflow_host_command_supervisor::HostCommandControl,
    ) -> archon_workflow::WorkflowResult<
        crate::command::workflow_host_command_supervisor::SupervisedProcessOutput,
    > {
        use archon_workflow::{
            GateEnvelopeV1, PreparedPublicationEntry, PreparedPublicationV1, RemediationScope,
        };

        let candidate =
            String::from_utf8_lossy(&request.stdin.clone().unwrap_or_default()).into_owned();
        let argument = |name: &str| {
            request
                .args
                .windows(2)
                .find(|pair| pair[0] == name)
                .map(|pair| std::path::PathBuf::from(&pair[1]))
                .unwrap()
        };
        let task_file = argument("--task-file");
        let task_context_file = argument("--task-context-file");
        if task_file != task_context_file {
            let staging_root = argument("--staging-root");
            assert_ne!(task_file, task_context_file);
            assert!(task_file.starts_with(staging_root));
            let staged_bytes = std::fs::read(&task_file).unwrap();
            assert_eq!(staged_bytes, candidate.as_bytes());
            self.staged_task_reads
                .lock()
                .unwrap()
                .push((task_file.clone(), staged_bytes));
        }
        if self.mutate_frozen_task {
            std::fs::write(task_file, b"child mutation during rejudge").unwrap();
        }
        if let Some(path) = &self.mutate_destination_on_execute {
            std::fs::write(path, b"disk changed during rejudge").unwrap();
        }
        self.executed.lock().unwrap().push(candidate.clone());
        let refused = self.refuse_first_two && self.executed.lock().unwrap().len() <= 2;
        let findings = refused
            .then(|| {
                vec![archon_workflow::GatePolicyFinding {
                    deterministic_defect: None,
                    text: "candidate needs another author round".into(),
                    subject: "TASK-X-010".into(),
                    source_path: None,
                    remediation_scope: RemediationScope::Body,
                }]
            })
            .unwrap_or_default();
        let envelope = GateEnvelopeV1 {
            schema_version: archon_workflow::GATE_ENVELOPE_SCHEMA_VERSION,
            report: serde_json::json!(if refused { "refused" } else { "accepted" }),
            policy_findings: findings,
            operational_error: None,
        };
        let envelope_bytes = serde_json::to_vec_pretty(&envelope)?;
        let mut entries = Vec::new();
        for path in &request.declared_write_set {
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            let bytes = if path
                .file_name()
                .is_some_and(|name| name == "gate-envelope.json")
            {
                envelope_bytes.clone()
            } else {
                candidate.as_bytes().to_vec()
            };
            std::fs::write(path, &bytes).unwrap();
            entries.push(PreparedPublicationEntry {
                relative_path: path.file_name().unwrap().to_string_lossy().into_owned(),
                byte_len: bytes.len() as u64,
                blake3: archon_workflow::task_set_contract::content_digest(&bytes),
            });
        }
        entries.sort_by(|a, b| a.relative_path.cmp(&b.relative_path));
        let call_id = request
            .args
            .windows(2)
            .find(|pair| pair[0] == "--call-id")
            .map(|pair| pair[1].clone())
            .unwrap();
        let manifest = PreparedPublicationV1 {
            schema_version: archon_workflow::PREPARED_PUBLICATION_SCHEMA_VERSION,
            call_id,
            command_id: request.command_id,
            entries,
        };
        let stdout = serde_json::to_vec(&manifest)?;
        let stdout_bytes = stdout.len() as u64;
        Ok(
            crate::command::workflow_host_command_supervisor::SupervisedProcessOutput {
                exit_code: Some(0),
                timed_out: false,
                stdout_bytes,
                stderr_bytes: 0,
                stdout_retained_bytes: stdout_bytes,
                stderr_retained_bytes: 0,
                stdout_truncated: false,
                stderr_truncated: false,
                stdout,
                stderr: Vec::new(),
                stdout_path: None,
                stderr_path: None,
            },
        )
    }
}

const REAL_TASK_SCRIPT: &str = r##"async function workflow(w) {
    const target = {taskId: "TASK-X-010", fileName: "TASK-X-010.md"};
    for (const [index] of [1, 2, 3].entries()) {
        const body = [
            "```yaml", "task_id: TASK-X-010", "title: Round 1",
            "complexity: low", "status: ready", "depends_on: []", "blocks: []",
            "implements: []", "required_env_keys: []", "required_tools: []",
            "deliverable_contracts: []", "```", "", "# Scope", "",
            "Round 1 body.", "", "# Focused Tests", "",
            "- `cargo test -p archon-workflow --lib`", ""
        ].join("\n");
        const outcome = await w.hostCommand("land-task-body", {stdin: body, target});
        const sentBack = (outcome.gateEnvelope?.policy_findings || [])
            .some((finding) => finding.remediation_scope === "body");
        if (!sentBack) return "Round " + (index + 1);
    }
    throw new Error("no round passed");
}"##;

#[tokio::test]
async fn a_landing_bump_replays_superseded_rounds_and_rejudges_only_the_current_body() {
    let run = Run::new();
    let launch = run.build(9);
    run.run(launch.clone(), SCRIPT).await;
    assert_eq!(launch.executed.lock().unwrap().len(), 3);
    // Version 13: every landing now has another key.
    let bumped = run.build(13);
    let output = run.run(bumped.clone(), SCRIPT).await;
    assert_eq!(output, serde_json::json!("round 3"));
    assert_eq!(
        *bumped.executed.lock().unwrap(),
        vec!["round 3".to_string()],
        "only the current body is judged again; rounds 1 and 2 replay"
    );
    // A third resume under the same logic executes nothing.
    let again = run.build(13);
    run.run(again.clone(), SCRIPT).await;
    assert!(again.executed.lock().unwrap().is_empty());
}

#[tokio::test]
async fn an_old_round_asked_alone_replays_and_never_runs_over_the_current_body() {
    let run = Run::new();
    run.run(run.build(9), SCRIPT).await;
    let bumped = run.build(13);
    let outcome = run.run(bumped.clone(), ROUND_ONE).await;
    assert!(
        bumped.executed.lock().unwrap().is_empty(),
        "a superseded round must not run, so it publishes nothing"
    );
    assert_eq!(outcome["gateEnvelope"]["report"], "refused round 1");
}

/// An accepted superseded round cannot stand in for the subject's later
/// accepted landing: the answer must attest the current landing's bytes.
#[tokio::test]
async fn accepted_superseded_round_answers_with_the_current_landing_verdict() {
    let run = Run::new();
    const ACCEPTED_LANDINGS: &str = r#"async function workflow(w) {
        const target = {taskId: "TASK-X-001", fileName: "TASK-X-001.md"};
        for (const body of ["old accepted", "current accepted"]) {
            await w.hostCommand("land-task-body", {stdin: body, target});
        }
        return "seeded";
    }"#;
    const REPLAY_OLD: &str = r#"async function workflow(w) {
        const target = {taskId: "TASK-X-001", fileName: "TASK-X-001.md"};
        return await w.hostCommand("land-task-body", {stdin: "old accepted", target});
    }"#;
    run.run(run.build(9), ACCEPTED_LANDINGS).await;
    let host = run.build(9);
    let outcome = run.run(host.clone(), REPLAY_OLD).await;
    assert_eq!(outcome["gateEnvelope"]["report"], "current accepted");
    assert!(host.executed.lock().unwrap().is_empty());
}

#[tokio::test]
async fn stale_current_landing_is_rejudged_once_then_replayed() {
    let run = Run::new();
    const ACCEPTED_LANDINGS: &str = r#"async function workflow(w) {
        const target = {taskId: "TASK-X-001", fileName: "TASK-X-001.md"};
        for (const body of ["old accepted", "current accepted"]) {
            await w.hostCommand("land-task-body", {stdin: body, target});
        }
        return "seeded";
    }"#;
    const REPLAY_OLD: &str = r#"async function workflow(w) {
        const target = {taskId: "TASK-X-001", fileName: "TASK-X-001.md"};
        return await w.hostCommand("land-task-body", {stdin: "old accepted", target});
    }"#;
    run.run(run.build(9), ACCEPTED_LANDINGS).await;
    let first_resume = run.build(13);
    let outcome = run.run(first_resume.clone(), REPLAY_OLD).await;
    assert_eq!(outcome["gateEnvelope"]["report"], "current accepted");
    assert_eq!(
        first_resume.executed.lock().unwrap().as_slice(),
        ["current accepted"]
    );

    let second_resume = run.build(13);
    run.run(second_resume.clone(), REPLAY_OLD).await;
    assert!(second_resume.executed.lock().unwrap().is_empty());
}

#[tokio::test]
async fn without_a_bump_the_current_body_is_not_judged_again() {
    let run = Run::new();
    run.run(run.build(9), SCRIPT).await;
    let same = run.build(9);
    run.run(same.clone(), SCRIPT).await;
    assert!(same.executed.lock().unwrap().is_empty());
}

#[tokio::test]
async fn a_real_landing_host_and_resume_keep_the_accepted_task_bytes_after_a_logic_bump() {
    let run = Run::new();
    let context = run.context.clone();
    let task_path = context.task_root.join("TASK-X-010.md");
    crate::command::workflow_host_command_exec_tests::seed_frozen_chain(&context, &task_path);
    crate::command::workflow_decompose_state_tests::seed_state(
        &run.store,
        &run.run_id,
        &context.task_root.join(".decompose.log"),
    );
    let executed = Arc::new(Mutex::new(Vec::new()));
    let first = Arc::new(
        FixedHostCommandExecutor::with_process(
            crate::command::workflow_host_command_catalog::fixed_decomposition_catalog("rev-1")
                .unwrap(),
            context.clone(),
            run.store.run_dir(&run.run_id),
            Arc::new(RealLandingProcess {
                executed: Arc::clone(&executed),
                staged_task_reads: Arc::new(Mutex::new(Vec::new())),
                refuse_first_two: true,
                mutate_frozen_task: false,
                mutate_destination_on_execute: None,
            }),
        )
        .with_logic_version("land-task-body", Some(9)),
    );
    let first_lease = Arc::new(
        crate::command::workflow_executor_lease::acquire(
            &run.store.run_dir(&run.run_id),
            &run.run_id,
        )
        .unwrap(),
    );
    first_lease.record_executor().unwrap();
    let first_result = run
        .run_with_lease(first, REAL_TASK_SCRIPT, Some(first_lease))
        .await;
    assert_eq!(
        first_result,
        serde_json::json!("Round 3"),
        "executed={:?}; records={:?}",
        executed.lock().unwrap(),
        WorkflowV2ResultStore::new(run.store.run_dir(&run.run_id).join("v2"))
            .load_call_records()
            .unwrap()
            .iter()
            .map(|record| (&record.call.id, &record.status, &record.result.data))
            .collect::<Vec<_>>()
    );
    assert_eq!(executed.lock().unwrap().len(), 3);

    let files_before: Vec<(String, Vec<u8>)> = std::fs::read_dir(&context.task_root)
        .unwrap()
        .filter_map(Result::ok)
        .filter(|entry| entry.file_name().to_string_lossy().starts_with("TASK-X-"))
        .map(|entry| {
            (
                entry.file_name().to_string_lossy().into_owned(),
                std::fs::read(entry.path()).unwrap(),
            )
        })
        .collect();
    let v2 = WorkflowV2ResultStore::new(run.store.run_dir(&run.run_id).join("v2"));
    let before_records = v2.load_call_records().unwrap();
    let count_occurrence = |records: &[WorkflowV2CallRecord], occurrence: u64| {
        records
            .iter()
            .filter(|record| {
                archon_workflow::v2::script::landing_lineage::occurrence_of(record) == occurrence
                    && record
                        .call
                        .options
                        .host_command
                        .as_ref()
                        .is_some_and(|request| {
                            request
                                .stdin
                                .as_deref()
                                .is_some_and(|stdin| stdin.contains("Round 1"))
                        })
            })
            .count()
    };
    let before_round_1 = count_occurrence(&before_records, 1);
    let before_round_2 = count_occurrence(&before_records, 2);

    executed.lock().unwrap().clear();
    let bumped = Arc::new(
        FixedHostCommandExecutor::with_process(
            crate::command::workflow_host_command_catalog::fixed_decomposition_catalog("rev-2")
                .unwrap(),
            context.clone(),
            run.store.run_dir(&run.run_id),
            Arc::new(RealLandingProcess {
                executed: Arc::clone(&executed),
                staged_task_reads: Arc::new(Mutex::new(Vec::new())),
                refuse_first_two: false,
                mutate_frozen_task: false,
                mutate_destination_on_execute: None,
            }),
        )
        .with_logic_version("land-task-body", Some(13)),
    );
    let bumped_lease = Arc::new(
        crate::command::workflow_executor_lease::acquire(
            &run.store.run_dir(&run.run_id),
            &run.run_id,
        )
        .unwrap(),
    );
    bumped_lease.record_executor().unwrap();
    let resumed_result = run
        .run_with_lease(bumped, REAL_TASK_SCRIPT, Some(bumped_lease))
        .await;
    assert_eq!(resumed_result, serde_json::json!("Round 3"));
    let files_after: Vec<(String, Vec<u8>)> = std::fs::read_dir(&context.task_root)
        .unwrap()
        .filter_map(Result::ok)
        .filter(|entry| entry.file_name().to_string_lossy().starts_with("TASK-X-"))
        .map(|entry| {
            (
                entry.file_name().to_string_lossy().into_owned(),
                std::fs::read(entry.path()).unwrap(),
            )
        })
        .collect();
    assert_eq!(
        files_after, files_before,
        "every task file must remain byte-identical"
    );
    let resumed_executions = executed.lock().unwrap().clone();
    assert_eq!(
        resumed_executions.len(),
        1,
        "only the current body may execute"
    );
    assert!(resumed_executions[0].contains("title: Round 1"));
    let after_records = v2.load_call_records().unwrap();
    assert_eq!(count_occurrence(&after_records, 1), before_round_1);
    assert_eq!(count_occurrence(&after_records, 2), before_round_2);
}

#[path = "workflow_live_v2_upgrade_landing_child_write_tests.rs"]
mod child_write_tests;

#[path = "workflow_live_v2_upgrade_landing_race_tests.rs"]
mod race_tests;
#[path = "workflow_live_v2_upgrade_landing_receipt_tests.rs"]
mod receipt_tests;
#[path = "workflow_live_v2_upgrade_landing_restore_race_tests.rs"]
mod restore_race_tests;
