//! Issue 261: the fixed decomposition script through the real script host.
//! A body whose findings never improve pauses the run; a resume replays every
//! recorded attempt -- refused landings and failed author calls included --
//! verbatim back to the pause, passes it, and continues with new attempts.

use std::sync::Mutex;
use std::sync::atomic::AtomicBool;

use super::*;

/// Answers every author; content is unique per provider call, so each body
/// candidate is its own host-command identity.
struct CountingLlm {
    calls: AtomicUsize,
    repeat_body: AtomicBool,
    ids: Mutex<Vec<String>>,
    /// Body authors fail the way a dead provider does while this is set.
    fail_bodies: AtomicBool,
}

#[async_trait::async_trait]
impl WorkflowLlmClient for CountingLlm {
    async fn send_message(
        &self,
        _messages: Vec<serde_json::Value>,
        _system: Vec<serde_json::Value>,
        _tools: Vec<serde_json::Value>,
        _model: &str,
    ) -> archon_workflow::WorkflowResult<WorkflowAgentOutcome> {
        panic!("fixed authors must use raw run_agent")
    }

    async fn run_agent(
        &self,
        request: archon_workflow::WorkflowAgentCall,
    ) -> archon_workflow::WorkflowResult<WorkflowAgentOutcome> {
        let ordinal = self.calls.fetch_add(1, Ordering::SeqCst) + 1;
        self.ids.lock().unwrap().push(request.task.clone());
        if self.fail_bodies.load(Ordering::SeqCst)
            && request.task.contains("Author the complete TASK body")
        {
            return Err(WorkflowError::StageFailed(
                "agent transport failed: stream ended before message_stop".to_string(),
            ));
        }
        let content = if request.task.contains("Author ONLY entry AC-X-001") {
            serde_json::json!({"id": "AC-X-001"}).to_string()
        } else if self.repeat_body.load(Ordering::SeqCst)
            && request.task.contains("Author the complete TASK body")
        {
            "# identical body candidate".into()
        } else {
            format!("# candidate {ordinal}")
        };
        Ok(WorkflowAgentOutcome {
            content,
            stop_reason: Some("end_turn".into()),
            ..WorkflowAgentOutcome::default()
        })
    }
}

/// Every gate is clean except the body gate, which reports the same finding
/// until `fixed` is set: the operator's or a fix's change -- or a judge that
/// answers differently when asked again. With `refuse` set, a candidate with
/// findings is refused: no publication receipt, as the real freeze does.
struct StallingHost {
    fixed: AtomicBool,
    varying: AtomicBool,
    refuse: AtomicBool,
    /// The body gate reports an operational error while this is set.
    operational: AtomicBool,
    lands: AtomicUsize,
}

#[async_trait::async_trait]
impl crate::command::workflow_host_command_exec::WorkflowHostCommandExecutor for StallingHost {
    fn call_identity(
        &self,
        request: &archon_workflow::HostCommandRequest,
    ) -> archon_workflow::WorkflowResult<String> {
        Ok(format!(
            "stall-{}-{}",
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
        if request.command_id == "land-task-body" {
            self.lands.fetch_add(1, Ordering::SeqCst);
        }
        let findings =
            if request.command_id == "land-task-body" && !self.fixed.load(Ordering::SeqCst) {
                vec![archon_workflow::GatePolicyFinding {
                    deterministic_defect: None,
                    text: if self.varying.load(Ordering::SeqCst)
                        && self.lands.load(Ordering::SeqCst) == 1
                    {
                        "deliverable lacks observation and validation".into()
                    } else {
                        "deliverable `src/a.rs` carries no observation".into()
                    },
                    subject: "TASK-X-010".into(),
                    source_path: None,
                    remediation_scope: archon_workflow::RemediationScope::Body,
                }]
            } else {
                Vec::new()
            };
        let subjects = if request.command_id == "freeze-skeleton" {
            vec![archon_workflow::HostCommandSubject {
                task_id: "TASK-X-010".into(),
                file_name: "TASK-X-010.md".into(),
            }]
        } else {
            Vec::new()
        };
        let operational = (request.command_id == "land-task-body"
            && self.operational.load(Ordering::SeqCst))
        .then(|| archon_workflow::GateOperationalError {
            kind: "operational".into(),
            text: "judge response was truncated".into(),
        });
        let findings = if operational.is_some() {
            Vec::new()
        } else {
            findings
        };
        let refused =
            operational.is_some() || (!findings.is_empty() && self.refuse.load(Ordering::SeqCst));
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
                policy_findings: findings,
                operational_error: operational,
            }),
            publication_receipt: (!refused).then(|| archon_workflow::PublicationReceiptV1 {
                schema_version: archon_workflow::PUBLICATION_RECEIPT_SCHEMA_VERSION,
                call_id,
                command_id: request.command_id,
                entries: Vec::new(),
                committed_at: "2026-10-03T00:00:00Z".into(),
            }),
            subjects,
            postcondition: Some(archon_workflow::CommandPostconditionEvaluation {
                satisfied: true,
                summary: "fixture postcondition".into(),
            }),
        })
    }
}

fn fixed_args(root: &std::path::Path) -> serde_json::Value {
    serde_json::json!({
        "projectRoot": root,
        "repositoryRoot": root,
        "prdPath": root.join("PRD.md"),
        "prdDigest": "a".repeat(64),
        "acceptanceCriteria": {"AC-X-001": "example criterion"},
        "taskRoot": root.join("tasks"),
        "gateMode": "enforce"
    })
}

async fn run_fixed(
    temp: &tempfile::TempDir,
    store: &WorkflowStore,
    run_id: &str,
    llm: &Arc<CountingLlm>,
    host: &Arc<StallingHost>,
) -> Result<WorkflowV2ScriptSummary, WorkflowError> {
    let (runner, _rx) = runner(
        store,
        run_id,
        llm.clone(),
        Some(host.clone()),
        Some(fixed_args(temp.path())),
    );
    runner
        .run(crate::command::workflow_decompose::FIXED_SCRIPT_SOURCE)
        .await
}

fn fixture() -> (
    tempfile::TempDir,
    WorkflowStore,
    String,
    Arc<CountingLlm>,
    Arc<StallingHost>,
) {
    let (temp, store, run_id) = new_run();
    let llm = Arc::new(CountingLlm {
        calls: AtomicUsize::new(0),
        repeat_body: AtomicBool::new(false),
        ids: Mutex::new(Vec::new()),
        fail_bodies: AtomicBool::new(false),
    });
    let host = Arc::new(StallingHost {
        fixed: AtomicBool::new(false),
        varying: AtomicBool::new(false),
        refuse: AtomicBool::new(false),
        operational: AtomicBool::new(false),
        lands: AtomicUsize::new(0),
    });
    (temp, store, run_id, llm, host)
}

#[tokio::test]
async fn a_stalled_body_pauses_the_fixed_run_and_a_resume_after_a_fix_accepts_it() {
    let (temp, store, run_id, llm, host) = fixture();

    let error = run_fixed(&temp, &store, &run_id, &llm, &host)
        .await
        .expect_err("a body without progress pauses the run");
    assert!(
        matches!(error, WorkflowError::ControlPaused(_)),
        "{error:?}"
    );
    assert_eq!(
        store.load_state(&run_id).unwrap().status,
        archon_workflow::RunStatus::Paused
    );
    // Acceptance, skeleton, then one baseline body attempt and three without
    // progress.
    assert_eq!(llm.calls.load(Ordering::SeqCst), 6);
    let paused = pause_events(&store, &run_id);
    assert_eq!(paused.len(), 1, "{paused:?}");
    let detail = &paused[0].detail;
    assert_eq!(detail["pause_id"], "pause-body-TASK-X-010-1", "{detail}");
    assert_eq!(detail["evidence"]["subject"], "body-TASK-X-010", "{detail}");
    assert_eq!(detail["evidence"]["reason"], "no_progress", "{detail}");
    assert_eq!(detail["evidence"]["author_calls"], 4, "{detail}");
    assert!(
        detail["evidence"]["last_findings"][0]
            .as_str()
            .is_some_and(|text| text.contains("carries no observation")),
        "{detail}"
    );

    host.fixed.store(true, Ordering::SeqCst);
    resume(&store, &run_id);
    let summary = run_fixed(&temp, &store, &run_id, &llm, &host)
        .await
        .expect("the resumed run continues");

    assert_eq!(summary.status, WorkflowV2Status::Accepted, "{summary:?}");
    assert_eq!(
        llm.calls.load(Ordering::SeqCst),
        7,
        "every recorded author call is reused; only the attempt after the pause is new"
    );
    assert_eq!(pause_events(&store, &run_id).len(), 1);
}

#[tokio::test]
async fn a_resume_with_nothing_changed_makes_new_attempts_before_it_pauses_again() {
    let (temp, store, run_id, llm, host) = fixture();
    run_fixed(&temp, &store, &run_id, &llm, &host)
        .await
        .expect_err("first stall pauses");
    resume(&store, &run_id);

    let error = run_fixed(&temp, &store, &run_id, &llm, &host)
        .await
        .expect_err("still no progress");

    assert!(
        matches!(error, WorkflowError::ControlPaused(_)),
        "{error:?}"
    );
    assert_eq!(
        llm.calls.load(Ordering::SeqCst),
        9,
        "the resume grants a fresh window: three new attempts, not an immediate re-pause"
    );
    let paused = pause_events(&store, &run_id);
    assert_eq!(paused.len(), 2, "{paused:?}");
    assert_eq!(paused[1].detail["pause_id"], "pause-body-TASK-X-010-2");
    assert_eq!(paused[1].detail["evidence"]["author_calls"], 7);
}

/// A refused landing (no receipt) is the subject's last record before the
/// pause, so no later record supersedes it. Resume must replay it from its
/// record: re-asking a judge that now answers differently would rebuild a
/// different history and let the old pause pass a stall it never covered.
#[tokio::test]
async fn a_resume_replays_the_refused_landing_before_the_pause_instead_of_re_judging_it() {
    let (temp, store, run_id, llm, host) = fixture();
    host.refuse.store(true, Ordering::SeqCst);
    run_fixed(&temp, &store, &run_id, &llm, &host)
        .await
        .expect_err("refused candidates without progress pause the run");
    assert_eq!(llm.calls.load(Ordering::SeqCst), 6);
    let lands_before = host.lands.load(Ordering::SeqCst);

    // The judge now accepts anything it is asked.
    host.fixed.store(true, Ordering::SeqCst);
    resume(&store, &run_id);
    let summary = run_fixed(&temp, &store, &run_id, &llm, &host)
        .await
        .expect("the resumed run continues");

    assert_eq!(summary.status, WorkflowV2Status::Accepted, "{summary:?}");
    assert_eq!(
        llm.calls.load(Ordering::SeqCst),
        7,
        "the recorded attempts replay to the pause, which passes; one new attempt follows"
    );
    assert_eq!(
        host.lands.load(Ordering::SeqCst),
        lands_before + 1,
        "only the new candidate is judged"
    );
    assert!(record_exists(&store, &run_id, "body-TASK-X-010-author-5"));
}

/// A failed author call (the provider never answered) is the last record
/// before an operational pause. Resume replays it as failed, reaches the
/// pause, and only then asks the provider again.
#[tokio::test]
async fn a_resume_replays_the_failed_author_call_before_the_pause_instead_of_re_asking() {
    let (temp, store, run_id, llm, host) = fixture();
    host.fixed.store(true, Ordering::SeqCst);
    llm.fail_bodies.store(true, Ordering::SeqCst);
    run_fixed(&temp, &store, &run_id, &llm, &host)
        .await
        .expect_err("an outage pauses the run");
    let paused = pause_events(&store, &run_id);
    assert_eq!(
        paused[0].detail["evidence"]["reason"],
        "operational_no_progress"
    );

    llm.fail_bodies.store(false, Ordering::SeqCst);
    resume(&store, &run_id);
    let summary = run_fixed(&temp, &store, &run_id, &llm, &host)
        .await
        .expect("the resumed run continues");

    assert_eq!(summary.status, WorkflowV2Status::Accepted, "{summary:?}");
    assert!(
        record_exists(&store, &run_id, "body-TASK-X-010-author-4"),
        "the three failed calls replayed; the first new call is the fourth"
    );
    assert_eq!(pause_events(&store, &run_id).len(), 1);
}

#[path = "workflow_live_v2_script_pause_occurrence_tests.rs"]
mod occurrences;
#[path = "workflow_live_v2_script_pause_operational_tests.rs"]
mod operational;
#[path = "workflow_live_v2_script_pause_fixed_resume_tests.rs"]
mod previous_binary_resume;
