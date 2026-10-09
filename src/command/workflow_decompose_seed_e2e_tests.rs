//! Issue 360 end to end: the fixed decomposition script on an earlier shape,
//! upgraded, resumed through the real script host from its phase seed. Only
//! the entries that fail or were refuted are authored again, then the gate.
use std::collections::BTreeSet;
use std::sync::Mutex;
use std::sync::atomic::AtomicBool;

use super::*;
use crate::command::workflow_decompose::FIXED_SCRIPT_SOURCE;
use crate::command::workflow_decompose_seed::{
    current_seed, seeded_arguments, tests::record_transitions,
};
use archon_workflow::WorkflowResult;

/// The same script before a prompt change: every author input differs.
fn earlier_script() -> String {
    let earlier = FIXED_SCRIPT_SOURCE
        .replace(
            "Author exactly one acceptance entry identified below",
            "Author one acceptance entry named below",
        )
        .replace(
            "for the frozen acceptance contract.",
            "for the frozen acceptance contract (earlier wording).",
        )
        .replace(
            "Preserve every frozen tuple field exactly.",
            "Preserve each frozen tuple field exactly.",
        );
    assert_eq!(earlier.matches("earlier wording").count(), 1);
    assert!(
        earlier.contains("Author one acceptance entry named below")
            && earlier.contains("Preserve each frozen")
    );
    earlier
}

#[derive(Default)]
struct Authors {
    /// AC-2 is authored weak, and the judge refutes it, until this is set.
    strong: AtomicBool,
    weak_body: AtomicBool,
    tasks: Mutex<Vec<String>>,
}

impl Authors {
    fn tasks(&self) -> Vec<String> {
        self.tasks.lock().unwrap().clone()
    }
}

fn entry_text(id: &str, command: &str) -> String {
    serde_json::json!({"id": id, "criterion": "c", "check": {"kind": "command", "command": command, "cwd": "project_root"},
        "gap_permitted": false, "covers": [], "judgment": {"verdict": "accepted", "counterexample": "", "reason": "", "host_call_id": ""}})
    .to_string()
}

#[async_trait::async_trait]
impl WorkflowLlmClient for Authors {
    async fn send_message(
        &self,
        _: Vec<serde_json::Value>,
        _: Vec<serde_json::Value>,
        _: Vec<serde_json::Value>,
        _: &str,
    ) -> WorkflowResult<WorkflowAgentOutcome> {
        panic!("fixed authors use raw run_agent")
    }
    async fn run_agent(
        &self,
        request: archon_workflow::WorkflowAgentCall,
    ) -> WorkflowResult<WorkflowAgentOutcome> {
        self.tasks.lock().unwrap().push(request.task.clone());
        let entry = request
            .task
            .split("Author ONLY entry ")
            .nth(1)
            .and_then(|rest| rest.split(':').next());
        let content = match entry {
            Some("AC-2") if !self.strong.load(Ordering::SeqCst) => entry_text("AC-2", "weak"),
            Some(id) => entry_text(id, &format!("check {id}")),
            None if request.task.contains("task-skeleton") => "skeleton candidate".into(),
            None if self.weak_body.load(Ordering::SeqCst) => "weak body".into(),
            None => "good body".into(),
        };
        Ok(WorkflowAgentOutcome {
            content,
            stop_reason: Some("end_turn".into()),
            ..WorkflowAgentOutcome::default()
        })
    }
}

/// Refutes a weak entry or body; every other gate is clean. `outage` makes
/// the acceptance gate unable to judge.
#[derive(Default)]
struct Judge {
    outage: AtomicBool,
    runs: Mutex<Vec<String>>,
}

impl Judge {
    fn runs(&self, command: &str) -> usize {
        self.runs
            .lock()
            .unwrap()
            .iter()
            .filter(|c| *c == command)
            .count()
    }
}

#[async_trait::async_trait]
impl crate::command::workflow_host_command_exec::WorkflowHostCommandExecutor for Judge {
    fn call_identity(
        &self,
        request: &archon_workflow::HostCommandRequest,
    ) -> WorkflowResult<String> {
        let stdin = request.stdin.as_deref().unwrap_or_default().as_bytes();
        Ok(format!(
            "judge-{}-{}",
            request.command_id,
            archon_workflow::task_set_contract::content_digest(stdin)
        ))
    }
    fn logic_version(
        &self,
        request: &archon_workflow::HostCommandRequest,
    ) -> WorkflowResult<Option<u32>> {
        Ok(crate::command::workflow_host_command_logic::versions()
            .get(&request.command_id)
            .copied())
    }
    fn logic_digest(
        &self,
        request: &archon_workflow::HostCommandRequest,
    ) -> WorkflowResult<Option<String>> {
        Ok(crate::command::workflow_host_command_logic::digests()
            .get(&request.command_id)
            .map(|(digest, _)| digest.clone()))
    }
    /// The executor's own rule: only a clean committed outcome is reused.
    fn record_is_reusable(
        &self,
        record: &archon_workflow::WorkflowV2CallRecord,
    ) -> WorkflowResult<bool> {
        Ok(
            serde_json::from_value::<archon_workflow::HostCommandResult>(
                record.result.data.clone(),
            )
            .is_ok_and(|o| o.reusable()),
        )
    }
    async fn execute(
        &self,
        request: archon_workflow::HostCommandRequest,
        _: Option<u64>,
    ) -> WorkflowResult<archon_workflow::HostCommandResult> {
        self.runs.lock().unwrap().push(request.command_id.clone());
        let stdin = request.stdin.clone().unwrap_or_default();
        let finding = |text: String, subject: &str, scope| archon_workflow::GatePolicyFinding {
            deterministic_defect: None,
            text,
            subject: subject.into(),
            source_path: None,
            remediation_scope: scope,
        };
        let findings: Vec<_> = match request.command_id.as_str() {
            "freeze-acceptance" => {
                serde_json::from_str::<serde_json::Value>(&stdin).unwrap()["entries"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .filter(|e| e["check"]["command"] == "weak")
                    .map(|e| {
                        let id = e["id"].as_str().unwrap();
                        finding(
                            format!("check '{id}' was refuted by the host judge; reason: weak"),
                            id,
                            archon_workflow::RemediationScope::CandidateArtifact,
                        )
                    })
                    .collect()
            }
            "land-task-body" if stdin.contains("weak") => vec![finding(
                "body is weak".into(),
                "TASK-1",
                archon_workflow::RemediationScope::Body,
            )],
            _ => Vec::new(),
        };
        let operational = (request.command_id == "freeze-acceptance"
            && self.outage.load(Ordering::SeqCst))
        .then(|| archon_workflow::GateOperationalError {
            kind: "operational".into(),
            text: "judge unavailable".into(),
        });
        let findings = if operational.is_some() {
            Vec::new()
        } else {
            findings
        };
        let refused = operational.is_some() || !findings.is_empty();
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
                command_id: request.command_id.clone(),
                entries: Vec::new(),
                committed_at: "2026-10-06T00:00:00Z".into(),
            }),
            subjects: if request.command_id == "freeze-skeleton" {
                vec![archon_workflow::HostCommandSubject {
                    task_id: "TASK-1".into(),
                    file_name: "TASK-1.md".into(),
                }]
            } else {
                Vec::new()
            },
            postcondition: Some(archon_workflow::CommandPostconditionEvaluation {
                satisfied: true,
                summary: "fixture".into(),
            }),
        })
    }
}

struct Run {
    temp: tempfile::TempDir,
    store: WorkflowStore,
    run_id: String,
    authors: Arc<Authors>,
    judge: Arc<Judge>,
}

impl Run {
    fn new() -> Self {
        let (temp, store, run_id) = new_run();
        Self {
            temp,
            store,
            run_id,
            authors: Arc::default(),
            judge: Arc::default(),
        }
    }
    fn args(&self) -> serde_json::Value {
        let root = self.temp.path();
        serde_json::json!({"projectRoot": root, "repositoryRoot": root, "prdPath": root.join("PRD.md"), "prdDigest": "a".repeat(64),
            "acceptanceCriteria": {"AC-1": "one", "AC-2": "two", "AC-3": "three"}, "taskRoot": root.join("tasks"), "gateMode": "enforce"})
    }
    async fn run(
        &self,
        script: &str,
        args: serde_json::Value,
    ) -> Result<WorkflowV2ScriptSummary, WorkflowError> {
        let (runner, _rx) = runner(
            &self.store,
            &self.run_id,
            self.authors.clone(),
            Some(self.judge.clone()),
            Some(args),
        );
        runner.run(script).await
    }
    /// What `workflow decompose resume` does after recording `steps`.
    fn upgrade(&self, steps: &[(&str, &str)]) -> serde_json::Value {
        record_transitions(&self.store, &self.run_id, steps);
        let criteria = [("AC-1", "one"), ("AC-2", "two"), ("AC-3", "three")]
            .map(|(id, text)| (id.into(), text.into()))
            .into();
        let seed = current_seed(
            &self.store,
            &self.run_id,
            &self.temp.path().join(".decompose.log"),
            &criteria,
        )
        .unwrap();
        assert!(seed.is_some());
        resume(&self.store, &self.run_id);
        seeded_arguments(&self.args(), seed.as_ref())
    }
    fn calls(&self) -> BTreeSet<String> {
        WorkflowV2ResultStore::new(self.store.run_dir(&self.run_id).join("v2"))
            .load_call_records()
            .unwrap()
            .into_iter()
            .map(|r| r.call.id)
            .collect()
    }
}

fn acceptance_tasks(tasks: &[String]) -> Vec<String> {
    tasks
        .iter()
        .filter_map(|t| {
            t.split("Author ONLY entry ")
                .nth(1)?
                .split(':')
                .next()
                .map(String::from)
        })
        .collect()
}

/// The old runtime stalls on a refuted AC-2: four rounds, then a pause.
async fn stalled_on_a_refuted_entry() -> Run {
    let run = Run::new();
    let error = run
        .run(&earlier_script(), run.args())
        .await
        .expect_err("the stall pauses");
    assert!(
        matches!(error, WorkflowError::ControlPaused(_)),
        "{error:?}"
    );
    assert_eq!(
        acceptance_tasks(&run.authors.tasks()),
        ["AC-1", "AC-2", "AC-3", "AC-2", "AC-2", "AC-2"]
    );
    assert_eq!(
        pause_events(&run.store, &run.run_id)[0].detail["pause_id"],
        "pause-acceptance-1"
    );
    run.authors.strong.store(true, Ordering::SeqCst);
    run
}

#[tokio::test]
async fn an_upgraded_resume_re_authors_only_the_refuted_entry_then_runs_the_gate() {
    let run = stalled_on_a_refuted_entry().await;
    let freezes = run.judge.runs("freeze-acceptance");
    let before = run.calls();
    let tasks = run.authors.tasks().len();
    let args = run.upgrade(&[("new-script", "next-rev")]);
    // The resume's plan preview runs the seeded script too.
    archon_workflow::v2::script::dry_run_workflow_plan(FIXED_SCRIPT_SOURCE, Some(&args))
        .await
        .unwrap();
    let summary = run
        .run(FIXED_SCRIPT_SOURCE, args)
        .await
        .expect("the seeded run completes");
    assert_eq!(summary.status, WorkflowV2Status::Accepted, "{summary:?}");
    let new_tasks = run.authors.tasks()[tasks..].to_vec();
    assert_eq!(
        acceptance_tasks(&new_tasks),
        ["AC-2"],
        "AC-1 and AC-3 are carried"
    );
    assert!(
        new_tasks[0].contains("Author exactly one acceptance entry"),
        "the new prompt"
    );
    assert!(
        new_tasks[0].contains("was refuted by the host judge"),
        "with the last gate's finding"
    );
    let new: Vec<_> = run
        .calls()
        .difference(&before)
        .filter(|id| id.contains("author"))
        .cloned()
        .collect();
    assert_eq!(
        new,
        [
            "acceptance-author-AC-2-16",
            "body-TASK-1-author-1",
            "skeleton-author-1"
        ],
        "after every old ordinal"
    );
    assert_eq!(
        run.judge.runs("freeze-acceptance"),
        freezes + 1,
        "the gate judges the carried candidate once"
    );
}

#[tokio::test]
async fn a_seeded_round_interrupted_and_resumed_replays_its_own_calls() {
    let run = stalled_on_a_refuted_entry().await;
    run.judge.outage.store(true, Ordering::SeqCst);
    let args = run.upgrade(&[("new-script", "next-rev")]);
    let error = run
        .run(FIXED_SCRIPT_SOURCE, args)
        .await
        .expect_err("an outage pauses the seeded round");
    assert!(
        matches!(error, WorkflowError::ControlPaused(_)),
        "{error:?}"
    );
    let paused = pause_events(&run.store, &run.run_id);
    assert_eq!(
        paused.last().unwrap().detail["pause_id"],
        "pause-acceptance-2",
        "the old pause is not passed as taken"
    );
    // Issue 362: the freeze retried after each outage authors nothing more.
    assert_eq!(acceptance_tasks(&run.authors.tasks()[6..]), ["AC-2"]);
    assert!(run.calls().contains("acceptance-author-AC-2-16"));
    // The same runtime: the same seed, so the seeded calls answer from their records.
    run.judge.outage.store(false, Ordering::SeqCst);
    let tasks = run.authors.tasks().len();
    let freezes = run.judge.runs("freeze-acceptance");
    let args = run.upgrade(&[("new-script", "next-rev")]);
    let summary = run
        .run(FIXED_SCRIPT_SOURCE, args)
        .await
        .expect("the resumed seeded run completes");
    assert_eq!(summary.status, WorkflowV2Status::Accepted, "{summary:?}");
    assert!(
        acceptance_tasks(&run.authors.tasks()[tasks..]).is_empty(),
        "the seeded AC-2 reply answers from its record, and nothing is re-authored"
    );
    assert_eq!(
        run.judge.runs("freeze-acceptance"),
        freezes + 1,
        "after the pause, one freeze judges the seeded candidate"
    );
}

#[tokio::test]
async fn a_second_upgrade_during_the_seeded_round_starts_from_its_repairs() {
    let run = stalled_on_a_refuted_entry().await;
    run.judge.outage.store(true, Ordering::SeqCst);
    let args = run.upgrade(&[("new-script", "next-rev")]);
    run.run(FIXED_SCRIPT_SOURCE, args)
        .await
        .expect_err("an outage pauses the seeded round");
    run.judge.outage.store(false, Ordering::SeqCst);
    let tasks = run.authors.tasks().len();
    let freezes = run.judge.runs("freeze-acceptance");
    let args = run.upgrade(&[("new-script", "next-rev"), ("newer-script", "next-rev")]);
    assert_eq!(args["phaseSeed"]["transition_index"], 1);
    let script = FIXED_SCRIPT_SOURCE.replace(
        "Do not run commands or write files.",
        "Do not run any command or write any file.",
    );
    let summary = run
        .run(&script, args)
        .await
        .expect("the second seeded run completes");
    assert_eq!(summary.status, WorkflowV2Status::Accepted, "{summary:?}");
    assert!(
        acceptance_tasks(&run.authors.tasks()[tasks..]).is_empty(),
        "AC-2 was repaired in the first seeded round"
    );
    assert_eq!(run.judge.runs("freeze-acceptance"), freezes + 1);
}

#[path = "workflow_decompose_seed_history_e2e_tests.rs"]
mod history;

#[path = "workflow_decompose_seed_completed_phase_tests.rs"]
mod completed_phase_tests;
