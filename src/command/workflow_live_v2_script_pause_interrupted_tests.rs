//! Issue 375: a host command a pause stopped mid-flight answered nothing. Two
//! sibling candidates' attempts that both report no subject key alike, so the
//! earlier looks superseded by the later; a resume must still run each
//! interrupted call again, never replay its stub.

use super::*;

const TWO_LANDINGS_SCRIPT: &str = r#"
async function workflow(w) {
  const first = await w.hostCommand("land-candidate", { stdin: "candidate one" });
  const second = await w.hostCommand("land-candidate", { stdin: "candidate two" });
  return { first: first.interrupted === true, second: second.interrupted === true };
}
"#;

/// Refuses every candidate: no subject, no publication receipt.
struct RefusingHost {
    runs: AtomicUsize,
}

#[async_trait::async_trait]
impl crate::command::workflow_host_command_exec::WorkflowHostCommandExecutor for RefusingHost {
    fn call_identity(
        &self,
        request: &archon_workflow::HostCommandRequest,
    ) -> archon_workflow::WorkflowResult<String> {
        Ok(format!(
            "refuse-{}-{}",
            request.command_id,
            archon_workflow::task_set_contract::content_digest(
                request.stdin.as_deref().unwrap_or_default().as_bytes()
            )
        ))
    }

    fn record_is_reusable(
        &self,
        _record: &archon_workflow::WorkflowV2CallRecord,
    ) -> archon_workflow::WorkflowResult<bool> {
        Ok(true)
    }

    async fn execute(
        &self,
        _request: archon_workflow::HostCommandRequest,
        _expected_generation: Option<u64>,
    ) -> archon_workflow::WorkflowResult<archon_workflow::HostCommandResult> {
        self.runs.fetch_add(1, Ordering::SeqCst);
        Ok(archon_workflow::HostCommandResult {
            exit_code: Some(1),
            stdout: String::new(),
            stderr: "refused".into(),
            stdout_bytes: 0,
            stderr_bytes: 7,
            timed_out: false,
            interrupted: false,
            stdout_truncated: false,
            stderr_truncated: false,
            gate_envelope: None,
            publication_receipt: None,
            subjects: Vec::new(),
            postcondition: None,
        })
    }
}

#[tokio::test]
async fn a_resume_runs_each_interrupted_sibling_landing_again() {
    let (_temp, store, run_id) = new_run();
    let host = Arc::new(RefusingHost {
        runs: AtomicUsize::new(0),
    });
    let (first_session, _rx) = runner(
        &store,
        &run_id,
        Arc::new(PanicLlm),
        Some(host.clone()),
        None,
    );
    first_session
        .run(TWO_LANDINGS_SCRIPT)
        .await
        .expect("the first session ends");
    assert_eq!(host.runs.load(Ordering::SeqCst), 2);

    // The disk as the issue found it: both landings stopped by a pause, each
    // a stub with no subject and no receipt.
    let v2 = WorkflowV2ResultStore::new(store.run_dir(&run_id).join("v2"));
    let mut slots: Vec<_> = v2
        .load_call_records()
        .unwrap()
        .into_iter()
        .filter(|record| record.call.method == WorkflowV2HostMethod::HostCommand)
        .collect();
    assert_eq!(slots.len(), 2, "{slots:?}");
    slots.sort_by(|a, b| a.started_at.cmp(&b.started_at));
    for record in &mut slots {
        record.status = WorkflowV2Status::NeedsReview;
        record.result.status = WorkflowV2Status::NeedsReview;
        record.result.data["interrupted"] = serde_json::json!("paused");
        let raw = serde_json::to_vec_pretty(&*record).unwrap();
        std::fs::write(v2.result_path(&record.call.id), raw).unwrap();
    }
    assert!(slots[0].started_at < slots[1].started_at);
    set_status(&store, &run_id, archon_workflow::RunStatus::Paused);
    resume(&store, &run_id);

    let (resumed, _rx) = runner(
        &store,
        &run_id,
        Arc::new(PanicLlm),
        Some(host.clone()),
        None,
    );
    resumed
        .run(TWO_LANDINGS_SCRIPT)
        .await
        .expect("the resume ends");
    assert_eq!(
        host.runs.load(Ordering::SeqCst),
        4,
        "each interrupted landing runs again once on resume"
    );
    for record in &slots {
        let now = v2.load_call_record(&record.call.id).unwrap().unwrap();
        assert!(
            now.result.data["interrupted"] != "paused",
            "the stub is replaced by a real answer: {:?}",
            now.result.data
        );
    }
}

const LANDING_THEN_PAUSE_SCRIPT: &str = r#"
async function workflow(w) {
  await w.hostCommand("land-candidate", { stdin: "candidate one" });
  await w.pause("pause-landing-1", { subject: "landing", reason: "no_progress" });
  return {};
}
"#;

/// Overwrites the one host-command slot as a pause-interrupted stub of the
/// same attempt and input, the way a late interruption save would.
fn stub_the_landing(store: &WorkflowStore, run_id: &str) -> WorkflowV2CallRecord {
    let v2 = WorkflowV2ResultStore::new(store.run_dir(run_id).join("v2"));
    let mut record = v2
        .load_call_records()
        .unwrap()
        .into_iter()
        .find(|record| record.call.method == WorkflowV2HostMethod::HostCommand)
        .expect("the landing left a record");
    record.status = WorkflowV2Status::NeedsReview;
    record.result.status = WorkflowV2Status::NeedsReview;
    record.result.data["interrupted"] = serde_json::json!("notification_delivery_failed");
    std::fs::write(
        v2.result_path(&record.call.id),
        serde_json::to_vec_pretty(&record).unwrap(),
    )
    .unwrap();
    record
}

/// Review finding 1: a pause covers the attempt its slot held, and that same
/// attempt later reads as interrupted. The pause credit replays nothing an
/// interruption left: the call runs again.
#[tokio::test]
async fn a_covered_attempt_later_marked_interrupted_runs_again() {
    let (_temp, store, run_id) = new_run();
    let host = Arc::new(RefusingHost {
        runs: AtomicUsize::new(0),
    });
    let (first_session, _rx) = runner(
        &store,
        &run_id,
        Arc::new(PanicLlm),
        Some(host.clone()),
        None,
    );
    let error = first_session
        .run(LANDING_THEN_PAUSE_SCRIPT)
        .await
        .expect_err("the script pauses");
    assert!(
        matches!(error, WorkflowError::ControlPaused(_)),
        "{error:?}"
    );
    assert_eq!(host.runs.load(Ordering::SeqCst), 1);
    stub_the_landing(&store, &run_id);
    resume(&store, &run_id);

    let (resumed, _rx) = runner(
        &store,
        &run_id,
        Arc::new(PanicLlm),
        Some(host.clone()),
        None,
    );
    let _ = resumed.run(LANDING_THEN_PAUSE_SCRIPT).await;
    assert_eq!(
        host.runs.load(Ordering::SeqCst),
        2,
        "the interrupted covered attempt runs again, never replays"
    );
}

/// Publishes every candidate: a committed receipt each time it runs.
struct PublishingHost {
    publishes: AtomicUsize,
}

#[async_trait::async_trait]
impl crate::command::workflow_host_command_exec::WorkflowHostCommandExecutor for PublishingHost {
    fn call_identity(
        &self,
        request: &archon_workflow::HostCommandRequest,
    ) -> archon_workflow::WorkflowResult<String> {
        Ok(format!(
            "publish-{}-{}",
            request.command_id,
            archon_workflow::task_set_contract::content_digest(
                request.stdin.as_deref().unwrap_or_default().as_bytes()
            )
        ))
    }

    fn record_is_reusable(
        &self,
        _record: &archon_workflow::WorkflowV2CallRecord,
    ) -> archon_workflow::WorkflowResult<bool> {
        Ok(true)
    }

    async fn execute(
        &self,
        request: archon_workflow::HostCommandRequest,
        _expected_generation: Option<u64>,
    ) -> archon_workflow::WorkflowResult<archon_workflow::HostCommandResult> {
        self.publishes.fetch_add(1, Ordering::SeqCst);
        let call_id = self.call_identity(&request)?;
        Ok(archon_workflow::HostCommandResult {
            exit_code: Some(0),
            stdout: String::new(),
            stderr: String::new(),
            stdout_bytes: 0,
            stderr_bytes: 0,
            timed_out: false,
            interrupted: false,
            stdout_truncated: false,
            stderr_truncated: false,
            gate_envelope: Some(archon_workflow::GateEnvelopeV1 {
                schema_version: archon_workflow::GATE_ENVELOPE_SCHEMA_VERSION,
                report: serde_json::json!("judged"),
                policy_findings: Vec::new(),
                operational_error: None,
            }),
            publication_receipt: Some(archon_workflow::PublicationReceiptV1 {
                schema_version: archon_workflow::PUBLICATION_RECEIPT_SCHEMA_VERSION,
                call_id,
                command_id: request.command_id,
                entries: Vec::new(),
                committed_at: "2026-10-09T00:00:00Z".into(),
            }),
            subjects: vec![archon_workflow::HostCommandSubject {
                task_id: "TASK-A".into(),
                file_name: "TASK-A.md".into(),
            }],
            postcondition: Some(archon_workflow::CommandPostconditionEvaluation {
                satisfied: true,
                summary: "fixture postcondition".into(),
            }),
        })
    }
}

const ONE_LANDING_SCRIPT: &str = r#"
async function workflow(w) {
  const landed = await w.hostCommand("land-candidate", { stdin: "candidate one" });
  return { landed: Boolean(landed.publicationReceipt) };
}
"#;

/// Review finding 2: a committed publication, then the delivery failure of
/// the same attempt saves its interruption. The committed record stays the
/// call's answer, so a resume reuses its receipt and lands nothing twice.
#[tokio::test]
async fn a_delivery_failure_after_a_commit_never_lands_twice() {
    let (_temp, store, run_id) = new_run();
    let host = Arc::new(PublishingHost {
        publishes: AtomicUsize::new(0),
    });
    let (first_session, _rx) = runner(
        &store,
        &run_id,
        Arc::new(PanicLlm),
        Some(host.clone()),
        None,
    );
    first_session
        .run(ONE_LANDING_SCRIPT)
        .await
        .expect("the first session lands");
    assert_eq!(host.publishes.load(Ordering::SeqCst), 1);
    let v2 = WorkflowV2ResultStore::new(store.run_dir(&run_id).join("v2"));
    let committed = v2
        .load_call_records()
        .unwrap()
        .into_iter()
        .find(|record| record.call.method == WorkflowV2HostMethod::HostCommand)
        .expect("the landing left a record");
    assert!(!committed.result.data["publicationReceipt"].is_null());

    // The same attempt's late delivery failure records its interruption.
    let (session, _ui) = runner(
        &store,
        &run_id,
        Arc::new(PanicLlm),
        Some(host.clone()),
        None,
    );
    let script_host = WorkflowScriptHost {
        host_occurrences: Default::default(),
        scaffold_hash: "late-delivery".into(),
        envelope_shape: ScriptEnvelopeShape::Compat,
        runner: session,
        accumulator: Arc::new(tokio::sync::Mutex::new(WorkflowScriptAccumulator::default())),
        tool_host: std::sync::OnceLock::new(),
        tool_budget: Default::default(),
    };
    let execution = WorkflowV2CallExecution {
        call: committed.call.clone(),
        input: serde_json::Value::Null,
        depends_on: committed.depends_on.clone(),
    };
    script_host
        .save_interrupted_call_record(
            &execution,
            "notification_delivery_failed",
            &WorkflowError::NotificationDelivery("delivery failed".into()),
            std::time::Duration::from_secs(1),
            committed.attempt,
            &committed.input_hash,
            None,
            None,
        )
        .await
        .expect("a kept commit is not an error");
    assert_eq!(
        v2.load_call_record(&committed.call.id).unwrap().unwrap(),
        committed,
        "the interruption never overwrites the committed result"
    );
    drop(script_host);

    set_status(&store, &run_id, archon_workflow::RunStatus::Paused);
    resume(&store, &run_id);
    let (resumed, _rx) = runner(
        &store,
        &run_id,
        Arc::new(PanicLlm),
        Some(host.clone()),
        None,
    );
    resumed
        .run(ONE_LANDING_SCRIPT)
        .await
        .expect("the resume ends");
    assert_eq!(
        host.publishes.load(Ordering::SeqCst),
        1,
        "the resume reuses the committed receipt and lands nothing twice"
    );
}
