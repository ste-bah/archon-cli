//! A silent fidelity critic pauses resumably; retries reuse the saved batches.
//!
//! Fixtures are a made-up PRD in a made-up domain: three tasks, each the
//! only claimant of its own two requirements, so the audit asks exactly
//! three call batches (one per cluster) and each can be counted.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering::SeqCst};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use archon_workflow::error::WorkflowResult;
use archon_workflow::llm_client_port::{WorkflowAgentOutcome, WorkflowLlmClient};
use async_trait::async_trait;

use super::*;
use crate::command::topology_lint::LintSource;
use crate::command::topology_lint::fidelity::evaluate_lint_with_fidelity_resumable;
use crate::command::workflow_freeze_budget::{Clock, FreezeBudget};
use crate::command::workflow_gate::GateEvaluation;
use crate::command::workflow_host_command_operational::{
    NextStep, OperationalAttempt, OperationalKind, classify, next_step, reported_progress,
};
use crate::command::workflow_host_command_supervisor::SupervisedProcessOutput;
use anyhow::Result;

const PREFIX: &str = "REQ-QX-";

/// Answers every obligation its prompt carries as necessarily true, and
/// records which ids each call asked. After `jump_after` calls it moves the
/// clock far past any deadline, as a provider that took the whole budget.
struct Critic {
    asked: Mutex<Vec<Vec<String>>>,
    jump: Option<(usize, Arc<AtomicU64>)>,
    calls: AtomicUsize,
    model: &'static str,
    /// Never answer: a provider that hangs.
    hang: bool,
    /// The request settings the client reports.
    request: Option<String>,
}

impl Critic {
    fn new() -> Arc<Self> {
        Self::build(None, "critic-model")
    }

    fn jumping_after(calls: usize, now: Arc<AtomicU64>) -> Arc<Self> {
        Self::build(Some((calls, now)), "critic-model")
    }

    fn build(jump: Option<(usize, Arc<AtomicU64>)>, model: &'static str) -> Arc<Self> {
        Arc::new(Self {
            asked: Mutex::new(Vec::new()),
            jump,
            calls: AtomicUsize::new(0),
            model,
            hang: false,
            request: Some("test-envelope".into()),
        })
    }

    fn calls(&self) -> usize {
        self.calls.load(SeqCst)
    }

    fn asked(&self) -> Vec<Vec<String>> {
        self.asked.lock().unwrap().clone()
    }
}

fn obligation_ids(prompt: &str) -> Vec<String> {
    let mut ids: Vec<String> = prompt
        .match_indices("\"id\":\"")
        .filter_map(|(at, marker)| {
            let rest = &prompt[at + marker.len()..];
            let id = &rest[..rest.find('"')?];
            id.starts_with(PREFIX).then(|| id.to_string())
        })
        .collect();
    ids.sort();
    ids.dedup();
    ids
}

#[async_trait]
impl WorkflowLlmClient for Critic {
    async fn send_message(
        &self,
        _messages: Vec<serde_json::Value>,
        _system: Vec<serde_json::Value>,
        _tools: Vec<serde_json::Value>,
        _model: &str,
    ) -> WorkflowResult<WorkflowAgentOutcome> {
        unreachable!("the audit pins temperature 0.0")
    }

    async fn send_message_with_temperature(
        &self,
        messages: Vec<serde_json::Value>,
        _system: Vec<serde_json::Value>,
        _tools: Vec<serde_json::Value>,
        _model: &str,
        _temperature: f64,
    ) -> WorkflowResult<WorkflowAgentOutcome> {
        let ids = obligation_ids(messages[0]["content"].as_str().unwrap());
        self.asked.lock().unwrap().push(ids.clone());
        let calls = self.calls.fetch_add(1, SeqCst) + 1;
        if self.hang {
            tokio::time::sleep(Duration::from_secs(10_000_000)).await;
        }
        if let Some((after, now)) = &self.jump
            && calls == *after
        {
            now.store(99_999, SeqCst);
        }
        if self.jump.as_ref().is_some_and(|(after, _)| calls > *after) {
            std::future::pending::<()>().await;
        }
        let verdicts: Vec<_> = ids
            .iter()
            .map(|id| serde_json::json!({"obligation_id": id, "necessarily_true": true, "reason": "obliged"}))
            .collect();
        Ok(WorkflowAgentOutcome {
            content: serde_json::json!({ "verdicts": verdicts }).to_string(),
            stop_reason: Some("end_turn".into()),
            ..WorkflowAgentOutcome::default()
        })
    }

    fn resolve_model_alias(&self, _model: &str) -> String {
        self.model.to_string()
    }

    fn message_request_identity(
        &self,
        _request: &archon_llm::provider::LlmRequest,
    ) -> Option<String> {
        self.request.clone()
    }

    fn request_identity(&self) -> Option<String> {
        self.request.clone()
    }
}

fn task(n: usize, claims: [usize; 2]) -> String {
    format!(
        "# TASK-QX-00{n} — Part {n}\n\n```yaml\ntask_id: TASK-QX-00{n}\ntitle: Part {n}\ncomplexity: medium\nstatus: pending\ndepends_on: []\nblocks: []\nimplements: [\"{PREFIX}00{}\", \"{PREFIX}00{}\"]\nrequired_env_keys: []\nrequired_tools: [cargo]\ndeliverable_contracts: []\n```\n\n## Scope\n\nBuilds part {n} of the gadget catalogue.\n\n## Focused Tests\n\n- `cargo test -p gadgets`\n",
        claims[0], claims[1]
    )
}

/// Three tasks, three clusters, three call batches. Returns the cwd.
fn corpus() -> tempfile::TempDir {
    let temp = tempfile::tempdir().expect("tempdir");
    let tasks = temp.path().join("tasks").join("PRD-QX-001");
    std::fs::create_dir_all(&tasks).expect("tasks dir");
    let mut prd = String::from("# Gadgets\n\n## Requirements\n\n");
    for n in 1..=6 {
        prd.push_str(&format!(
            "- {PREFIX}00{n}: the catalogue lists gadget {n}\n"
        ));
    }
    std::fs::write(temp.path().join("tasks/PRD-QX-001.md"), prd).expect("prd");
    for (n, claims) in [(1, [1, 2]), (2, [3, 4]), (3, [5, 6])] {
        std::fs::write(tasks.join(format!("TASK-QX-00{n}.md")), task(n, claims)).expect("task");
    }
    temp
}

fn hand_clock() -> (Arc<AtomicU64>, Clock) {
    let start = Instant::now();
    let seconds = Arc::new(AtomicU64::new(0));
    let read = seconds.clone();
    (
        seconds,
        Arc::new(move || start + Duration::from_secs(read.load(SeqCst))),
    )
}

fn lint_wall_clock() -> u64 {
    crate::command::workflow_host_command_catalog::fixed_decomposition_catalog("")
        .expect("catalog")
        .capabilities["task-set-lint"]
        .timeout_secs
}

/// The staged set gate's resume on `clock`: its catalog no-progress window.
fn staged(clock: Clock) -> FreezeResume {
    FreezeResume::saving(FreezeBudget::within(lint_wall_clock(), clock), true)
}

async fn lint(cwd: &Path, critic: Arc<Critic>, resume: &FreezeResume) -> Result<GateEvaluation> {
    evaluate_lint_with_fidelity_resumable(
        cwd,
        &LintSource::Tasks(cwd.join("tasks").join("PRD-QX-001")),
        archon_core::config::GateMode::Enforce,
        Ok(critic),
        &[],
        resume,
    )
    .await
}

async fn complete(cwd: &Path, critic: Arc<Critic>, resume: &FreezeResume) -> GateEvaluation {
    let evaluation = lint(cwd, critic, resume).await.expect("a complete lint");
    assert!(
        evaluation.operational_error().is_none(),
        "{:?}",
        evaluation.operational_error()
    );
    evaluation
}

fn records(cwd: &Path) -> Vec<PathBuf> {
    let dir = cwd.join(".archon/lint-cache/fidelity");
    let mut paths: Vec<PathBuf> = std::fs::read_dir(dir)
        .map(|entries| {
            entries
                .map(|entry| entry.unwrap().path())
                .filter(|path| path.extension().is_some_and(|ext| ext == "json"))
                .collect()
        })
        .unwrap_or_default();
    paths.sort();
    paths
}

fn ids(range: &[usize]) -> Vec<String> {
    range.iter().map(|n| format!("{PREFIX}00{n}")).collect()
}

/// A stalled critic exits 75; saved batches are on disk and reused on retry.
#[tokio::test(start_paused = true)]
async fn a_lint_stalled_on_the_provider_exits_resumable_and_a_retry_reuses_its_units() {
    let temp = corpus();
    let cwd = temp.path();
    let (now, clock) = hand_clock();
    let first = Critic::jumping_after(2, now);
    let resume = staged(clock);
    let error = match lint(cwd, first.clone(), &resume).await {
        Ok(evaluation) => panic!(
            "the silent provider must stop the lint: {:?} {}",
            evaluation.operational_error(),
            evaluation.report
        ),
        Err(error) => error,
    };
    let incomplete = LintIncomplete::caused(&error).expect("an incomplete, resumable lint");
    assert!(
        incomplete
            .to_string()
            .starts_with(LINT_INCOMPLETE_RESUMABLE),
        "{incomplete}"
    );
    let (stderr, status) = resumable_exit(&error).expect("the resumable exit");
    assert_eq!(status, 75);
    let output = SupervisedProcessOutput {
        exit_code: Some(status),
        timed_out: false,
        stdout: Vec::new(),
        stderr: stderr.clone().into_bytes(),
        stdout_bytes: 0,
        stderr_bytes: stderr.len() as u64,
    };
    assert_eq!(
        classify(&output),
        Some(OperationalKind::IncompleteResumable)
    );
    assert_eq!(reported_progress(stderr.as_bytes()), Some(2), "{stderr}");
    assert_eq!(
        first.calls(),
        3,
        "the next call starts and stops only on inactivity"
    );
    assert_eq!(records(cwd).len(), 2, "each finished batch is saved");

    let retry = Critic::new();
    let resume = staged(hand_clock().1);
    let evaluation = complete(cwd, retry.clone(), &resume).await;
    assert_eq!(retry.calls(), 1, "only the batch with no verdict is asked");
    let done: Vec<String> = first.asked()[..2].concat();
    assert!(
        retry.asked()[0].iter().all(|id| !done.contains(id)),
        "no saved unit is recomputed: {:?} after {:?}",
        retry.asked(),
        first.asked()
    );
    assert_eq!(resume.progress.total(), 3, "two reused, one saved");
    assert!(
        evaluation.report.contains("1 asked of") && evaluation.report.contains("2 served from"),
        "{}",
        evaluation.report
    );
    assert_eq!(records(cwd).len(), 3);
}

/// A silent provider stalls under both staged and ordinary invocation policies.
#[tokio::test(start_paused = true)]
async fn a_silent_provider_call_is_stopped_resumable() {
    let temp = corpus();
    let cwd = temp.path();
    let hanging = || {
        let mut critic = Critic::build(None, "critic-model");
        Arc::get_mut(&mut critic).expect("unshared").hang = true;
        critic
    };
    let (now, clock) = hand_clock();
    let resume = staged(clock);
    // Elapsed orchestration time cannot cut the provider's own idle window.
    now.store(lint_wall_clock() - 600 - 200, SeqCst);
    let critic = hanging();
    let error = lint(cwd, critic.clone(), &resume)
        .await
        .expect_err("stopped at the deadline");
    let text = LintIncomplete::caused(&error)
        .expect("resumable")
        .to_string();
    assert!(text.contains("3 batch(es) stalled"), "{text}");
    assert_eq!(critic.calls(), 3);
    let (stderr, status) = resumable_exit(&error).expect("resumable");
    assert_eq!(
        (status, reported_progress(stderr.as_bytes())),
        (75, Some(0))
    );
    assert!(records(cwd).is_empty());

    let error = lint(cwd, hanging(), &FreezeResume::none())
        .await
        .expect_err("a silent critic remains resumable");
    assert!(LintIncomplete::caused(&error).is_some());
}

/// Each attempt that saves a unit reports more progress than the last, so
/// the executor retries it; the last attempt finishes.
#[tokio::test(start_paused = true)]
async fn progress_grows_across_retries_until_the_lint_finishes() {
    let temp = corpus();
    let cwd = temp.path();
    let mut history = Vec::new();
    for attempt in 1..=2u32 {
        let (now, clock) = hand_clock();
        let error = lint(cwd, Critic::jumping_after(1, now), &staged(clock))
            .await
            .expect_err("one batch per attempt, then a silent provider");
        let (stderr, _) = resumable_exit(&error).expect("resumable");
        let progress = reported_progress(stderr.as_bytes());
        assert_eq!(progress, Some(u64::from(attempt)));
        history.push(OperationalAttempt {
            attempt,
            reason: OperationalKind::IncompleteResumable.label(),
            elapsed_secs: 0,
            progress,
        });
        assert_eq!(next_step(&history), NextStep::Retry);
    }
    let last = Critic::new();
    let resume = staged(hand_clock().1);
    complete(cwd, last.clone(), &resume).await;
    assert_eq!(last.calls(), 1);
    assert_eq!(resume.progress.total(), 3);
}

/// An edited task re-asks exactly its own cluster; an edited obligation
/// exactly the batch that carries it. Every other unit is reused.
#[tokio::test]
async fn a_changed_input_invalidates_exactly_the_affected_unit() {
    let temp = corpus();
    let cwd = temp.path();
    complete(cwd, Critic::new(), &FreezeResume::none()).await;
    let path = cwd.join("tasks/PRD-QX-001/TASK-QX-002.md");
    let edited = std::fs::read_to_string(&path).unwrap() + "\nAlso sorts the catalogue.\n";
    std::fs::write(&path, edited).unwrap();
    let critic = Critic::new();
    complete(cwd, critic.clone(), &FreezeResume::none()).await;
    assert_eq!(critic.asked(), vec![ids(&[3, 4])]);

    let prd = cwd.join("tasks/PRD-QX-001.md");
    let text = std::fs::read_to_string(&prd)
        .unwrap()
        .replace("lists gadget 5", "lists gadget 5 with its price");
    std::fs::write(&prd, text).unwrap();
    let critic = Critic::new();
    complete(cwd, critic.clone(), &FreezeResume::none()).await;
    assert_eq!(critic.asked(), vec![ids(&[5, 6])]);
}

/// A record that cannot be read, and one that reads but no longer passes
/// the reply validation, are both recomputed: the store is never trusted.
#[tokio::test]
async fn a_corrupt_or_forged_record_is_recomputed_never_trusted() {
    let temp = corpus();
    let cwd = temp.path();
    complete(cwd, Critic::new(), &FreezeResume::none()).await;
    let saved = records(cwd);
    assert_eq!(saved.len(), 3);
    std::fs::write(&saved[0], b"{\"schema\": 2, \"verd").unwrap();
    let mut forged: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&saved[1]).unwrap()).unwrap();
    let verdict = &mut forged["verdicts"][0];
    verdict["necessarily_true"] = false.into();
    verdict["weakest_task_id"] = "TASK-QX-001".into();
    verdict["quoted_task_text"] = "a sentence no task contains".into();
    std::fs::write(&saved[1], serde_json::to_vec(&forged).unwrap()).unwrap();
    let critic = Critic::new();
    let evaluation = complete(cwd, critic.clone(), &FreezeResume::none()).await;
    assert_eq!(critic.calls(), 2, "{:?}", critic.asked());
    assert!(
        !evaluation.report.contains("BLOCKING"),
        "a forged verdict never reaches the report: {}",
        evaluation.report
    );
    assert_eq!(records(cwd).len(), 3);
}

/// A verdict answered by another critic model, or by another binary, is
/// not reused: either can change what the same question gets.
#[tokio::test]
async fn a_saved_verdict_is_keyed_by_binary_and_critic_identity() {
    let temp = corpus();
    let cwd = temp.path();
    complete(cwd, Critic::new(), &FreezeResume::none()).await;
    let other_model = Critic::build(None, "another-model");
    complete(cwd, other_model.clone(), &FreezeResume::none()).await;
    assert_eq!(other_model.calls(), 3, "another model re-asks every batch");

    // Another endpoint or output ceiling (the client's request identity).
    let mut other_settings = Critic::build(None, "another-model");
    Arc::get_mut(&mut other_settings).expect("unshared").request = Some("e".repeat(64));
    complete(cwd, other_settings.clone(), &FreezeResume::none()).await;
    assert_eq!(other_settings.calls(), 3, "other request settings re-ask");

    let critic = Critic::new();
    let store = VerdictStore::new(store_dir(cwd), StoreIdentity::of(critic.as_ref()));
    let digest = "d".repeat(64);
    let verdicts = vec![archon_workflow::fidelity_audit::FidelityVerdict {
        obligation_id: format!("{PREFIX}001"),
        necessarily_true: true,
        weakest_task_id: String::new(),
        reason: "obliged".into(),
        quoted_task_text: String::new(),
    }];
    let obligations = vec![archon_workflow::fidelity_audit::ClaimedObligation {
        id: format!("{PREFIX}001"),
        text: "x".into(),
    }];
    store.save(&digest, &verdicts).expect("saved");
    assert_eq!(store.load(&digest, &obligations, &[]), Some(verdicts));
    let rebuilt = VerdictStore::new(
        store_dir(cwd),
        StoreIdentity::of(critic.as_ref()).with_binary("another-revision"),
    );
    assert_eq!(rebuilt.load(&digest, &obligations, &[]), None);
}

/// The staged set gate reads its own catalog no-progress window, never a copy.
#[test]
fn the_staged_set_gate_reads_its_catalog_no_progress_window() {
    let resume = FreezeResume::staged("task-set-lint");
    assert_eq!(resume.budget.outer_secs(), lint_wall_clock());
}

/// The binary's timeout semantics change the catalog identity; the fixed JS
/// was not edited. Rebuilding schema 1 reproduces the exact old catalog.
#[test]
fn issue356_catalog_identity_changes_but_script_does_not() {
    let catalog =
        crate::command::workflow_host_command_catalog::fixed_decomposition_catalog("rev").unwrap();
    let old_digest = "c59953c92b47b59b10c4b6b0cf19e6ae736c88aa037bef173cb141d25dd7b6c7";
    let mut old = catalog.clone();
    old.schema_version = 1;
    old.recompute_digest().unwrap();
    assert_eq!(old.digest, old_digest);
    assert_ne!(
        catalog.digest, old_digest,
        "timeout semantics must change resume identity"
    );
    assert_eq!(
        archon_workflow::workflow_scaffold_hash(
            crate::command::workflow_decompose::FIXED_SCRIPT_SOURCE
        ),
        "5564154c6205773b215bb3b54b390c981166e41e3384cfdd53b7f13b4b31f5a4"
    );
}

#[test]
fn fidelity_binary_identity_is_more_than_the_committed_revision() {
    let identity = StoreIdentity::of(Critic::new().as_ref());
    let value = serde_json::to_value(identity).unwrap();
    assert_ne!(
        value["binary"].as_str().unwrap(),
        env!("ARCHON_GIT_HASH"),
        "same-HEAD source edits, including parser edits, must invalidate verdicts"
    );
}

#[path = "fidelity_request_tests.rs"]
mod request_tests;
