//! #434: a saved verdict, true or false, is reused only for the same
//! obligation text over the same cluster with byte-identical task texts; an
//! edit re-asks exactly the obligations whose cluster holds the edited task;
//! two standing verdicts that disagree are one blocking finding quoting
//! both; a supporting quote that is no anchor is refused and asked again.

use std::collections::BTreeMap;

use super::*;
use crate::command::topology_lint::fidelity_store::{
    Question, StoreIdentity, VerdictStore, task_texts,
};
use archon_workflow::fidelity_audit::{FidelityVerdict, fidelity_cluster_digest};

/// Answers each obligation it is asked from a table, so the order in which
/// batches run never matters; records the ids of every call.
struct TableCritic {
    verdicts: Mutex<BTreeMap<String, serde_json::Value>>,
    asked: Mutex<Vec<Vec<String>>>,
}

impl TableCritic {
    fn new(verdicts: &[(&str, serde_json::Value)]) -> Arc<Self> {
        Arc::new(Self {
            verdicts: Mutex::new(
                verdicts
                    .iter()
                    .map(|(id, verdict)| (id.to_string(), verdict.clone()))
                    .collect(),
            ),
            asked: Mutex::new(Vec::new()),
        })
    }

    fn answer(&self, id: &str, verdict: serde_json::Value) {
        self.verdicts.lock().unwrap().insert(id.into(), verdict);
    }

    /// Every call's obligation ids, sorted, calls sorted.
    fn asked(&self) -> Vec<Vec<String>> {
        let mut asked = self.asked.lock().unwrap().clone();
        asked.sort();
        asked
    }

    fn reset(&self) {
        self.asked.lock().unwrap().clear();
    }
}

#[async_trait]
impl WorkflowLlmClient for TableCritic {
    fn message_request_identity(
        &self,
        _request: &archon_llm::provider::LlmRequest,
    ) -> Option<String> {
        Some("table-critic-envelope".into())
    }

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
        let prompt = messages[0]["content"].as_str().unwrap();
        let mut ids: Vec<String> = prompt
            .match_indices("{\"id\":\"")
            .map(|(at, marker)| {
                let rest = &prompt[at + marker.len()..];
                rest[..rest.find('"').unwrap()].to_string()
            })
            .collect();
        ids.sort();
        self.asked.lock().unwrap().push(ids.clone());
        let table = self.verdicts.lock().unwrap();
        let verdicts: Vec<_> = ids.iter().map(|id| table[id].clone()).collect();
        Ok(WorkflowAgentOutcome {
            content: serde_json::json!({ "verdicts": verdicts }).to_string(),
            stop_reason: Some("end_turn".into()),
            ..WorkflowAgentOutcome::default()
        })
    }
}

fn holds(id: &str, task: &str, quote: &str) -> serde_json::Value {
    serde_json::json!({"obligation_id": id, "necessarily_true": true, "reason": "obliged", "supporting_task_id": task, "supporting_text": quote})
}

fn gap(id: &str, task: &str, quote: &str) -> serde_json::Value {
    serde_json::json!({"obligation_id": id, "necessarily_true": false, "weakest_task_id": task, "reason": "a loophole", "quoted_task_text": quote})
}

fn task_path(cwd: &Path, id: &str) -> PathBuf {
    cwd.join(format!("tasks/PRD-WS-001/{id}.md"))
}

fn rewrite(path: &Path, from: &str, to: &str) {
    let text = std::fs::read_to_string(path).unwrap();
    assert!(text.contains(from), "{from} not in {}", path.display());
    std::fs::write(path, text.replace(from, to)).unwrap();
}

fn ids(list: &[&str]) -> Vec<String> {
    list.iter().map(|id| id.to_string()).collect()
}

/// A claimer-free task beside the corpus: in the set, in no cluster.
fn add_unrelated_task(cwd: &Path) {
    let body = super::body::candidate_body("Unrelated scope.")
        .replace("implements: [\"AC-WS-001\"]", "implements: []");
    std::fs::write(
        task_path(cwd, "TASK-WS-003"),
        body.replace("TASK-WS-002", "TASK-WS-003"),
    )
    .unwrap();
}

#[tokio::test]
async fn a_verdict_is_reused_only_while_its_cluster_texts_are_byte_identical() {
    let temp = corpus();
    let cwd = temp.path();
    add_unrelated_task(cwd);
    let critic = TableCritic::new(&[
        ("AC-WS-001", holds("AC-WS-001", "TASK-WS-001", SUPPORT)),
        ("G-WS-001", holds("G-WS-001", "TASK-WS-001", SUPPORT)),
    ]);
    evaluate(cwd, Ok(critic.clone())).await;
    assert_eq!(critic.asked(), vec![ids(&["AC-WS-001", "G-WS-001"])]);

    critic.reset();
    let unchanged = evaluate(cwd, Ok(critic.clone())).await;
    assert!(critic.asked().is_empty(), "unchanged: 0 asks");
    assert!(unchanged.report.contains("0 asked of") && unchanged.report.contains("2 served from"));

    rewrite(
        &task_path(cwd, "TASK-WS-003"),
        "Unrelated scope.",
        "Other scope.",
    );
    evaluate(cwd, Ok(critic.clone())).await;
    assert!(critic.asked().is_empty(), "a task in no cluster: 0 asks");

    // An edit outside the supporting quote still re-asks: a sentence added
    // anywhere in a claimer can be the loophole.
    rewrite(
        &task_path(cwd, "TASK-WS-001"),
        "## Focused Tests",
        "Entries may be skipped when the store is busy.\n\n## Focused Tests",
    );
    evaluate(cwd, Ok(critic.clone())).await;
    assert_eq!(critic.asked(), vec![ids(&["AC-WS-001", "G-WS-001"])]);

    critic.reset();
    let prd = cwd.join("tasks/PRD-WS-001.md");
    rewrite(
        &prd,
        "Ingest widgets into the shared store.",
        "Ingest widgets and gizmos.",
    );
    let evaluation = evaluate(cwd, Ok(critic.clone())).await;
    assert_eq!(
        critic.asked(),
        vec![ids(&["AC-WS-001", "G-WS-001"])],
        "a changed shared PRD source section changes both obligations' context"
    );
    assert!(
        !evaluation
            .findings
            .iter()
            .any(|finding| finding.text.starts_with("obligation ")),
        "{}",
        evaluation.report
    );
}

/// The body gate judged an obligation over the bodies landed so far; a
/// claimer that lands later with a loophole joins the cluster, so the set
/// gate asks again instead of reusing the earlier true verdict.
#[tokio::test]
async fn a_claimer_joining_the_cluster_re_asks_a_stored_true_verdict() {
    let temp = corpus();
    let cwd = temp.path();
    let critic = TableCritic::new(&[
        ("AC-WS-001", holds("AC-WS-001", "TASK-WS-001", SUPPORT)),
        ("G-WS-001", holds("G-WS-001", "TASK-WS-001", SUPPORT)),
    ]);
    evaluate(cwd, Ok(critic.clone())).await;
    critic.reset();
    std::fs::write(
        task_path(cwd, "TASK-WS-002"),
        super::body::candidate_body(super::body::HOLLOW),
    )
    .unwrap();
    critic.answer(
        "AC-WS-001",
        gap("AC-WS-001", "TASK-WS-002", super::body::HOLLOW),
    );
    let evaluation = evaluate(cwd, Ok(critic.clone())).await;
    assert_eq!(critic.asked(), vec![ids(&["AC-WS-001"])]);
    assert!(
        evaluation.report.contains("BLOCKING obligation AC-WS-001"),
        "{}",
        evaluation.report
    );
}

/// A false verdict rests on every task of its cluster: an edit to any
/// claimer re-asks it, and an edit re-asks only obligations whose cluster
/// holds the edited task.
#[tokio::test]
async fn a_false_verdict_is_reused_only_while_every_claimer_is_unchanged() {
    let temp = corpus();
    let cwd = temp.path();
    std::fs::write(
        task_path(cwd, "TASK-WS-002"),
        super::body::candidate_body(super::body::HOLLOW),
    )
    .unwrap();
    let critic = TableCritic::new(&[
        (
            "AC-WS-001",
            gap("AC-WS-001", "TASK-WS-002", super::body::HOLLOW),
        ),
        ("G-WS-001", holds("G-WS-001", "TASK-WS-001", SUPPORT)),
    ]);
    let first = evaluate(cwd, Ok(critic.clone())).await;
    assert_eq!(
        critic.asked(),
        vec![ids(&["AC-WS-001"]), ids(&["G-WS-001"])]
    );
    assert!(
        first.report.contains("BLOCKING obligation AC-WS-001"),
        "{}",
        first.report
    );

    critic.reset();
    let again = evaluate(cwd, Ok(critic.clone())).await;
    assert!(critic.asked().is_empty());
    assert!(
        again.report.contains("BLOCKING obligation AC-WS-001"),
        "the gap stays"
    );

    // TASK-WS-002 is in AC-WS-001's cluster only: only that is re-asked.
    rewrite(
        &task_path(cwd, "TASK-WS-002"),
        "## Focused Tests",
        "Also counts widgets.\n\n## Focused Tests",
    );
    evaluate(cwd, Ok(critic.clone())).await;
    assert_eq!(critic.asked(), vec![ids(&["AC-WS-001"])]);

    // TASK-WS-001 is in both clusters: both are re-asked.
    critic.reset();
    rewrite(
        &task_path(cwd, "TASK-WS-001"),
        LOOPHOLE,
        "Focused tests write the shared store.",
    );
    evaluate(cwd, Ok(critic.clone())).await;
    assert_eq!(
        critic.asked(),
        vec![ids(&["AC-WS-001"]), ids(&["G-WS-001"])]
    );
}

/// Two gates answered one obligation side by side and disagree on texts on
/// which both answers still hold: one blocking finding quoting both, no ask.
#[tokio::test]
async fn standing_verdicts_that_contradict_each_other_are_one_finding_quoting_both() {
    let temp = corpus();
    let cwd = temp.path();
    let critic = TableCritic::new(&[
        ("AC-WS-001", holds("AC-WS-001", "TASK-WS-001", SUPPORT)),
        ("G-WS-001", holds("G-WS-001", "TASK-WS-001", SUPPORT)),
    ]);
    evaluate(cwd, Ok(critic.clone())).await;
    // The other gate's answer, saved as it would be: same store identity,
    // same cluster texts.
    let tasks = vec![ClaimingTask {
        task_id: "TASK-WS-001".into(),
        text: std::fs::read_to_string(task_path(cwd, "TASK-WS-001")).unwrap(),
    }];
    let prd = std::fs::read_to_string(cwd.join("tasks/PRD-WS-001.md")).unwrap();
    let skeleton = archon_workflow::fidelity_audit::SkeletonSummary::absent()
        .with_prd_context_for(&prd, &["AC-WS-001".into(), "G-WS-001".into()]);
    let request = crate::command::topology_lint::fidelity_critic::request(&[], &[], &skeleton);
    let store = VerdictStore::new(store_dir(cwd), StoreIdentity::of(critic.as_ref()))
        .for_request(critic.message_request_identity(&request));
    let obligation = ClaimedObligation {
        id: "AC-WS-001".into(),
        text: "Ingestion stores a registry entry.".into(),
    };
    let texts = task_texts(&tasks);
    let refuted = FidelityVerdict {
        obligation_id: "AC-WS-001".into(),
        necessarily_true: false,
        weakest_task_id: "TASK-WS-001".into(),
        reason: "a loophole".into(),
        quoted_task_text: LOOPHOLE.into(),
        weakening_quotes: vec![LOOPHOLE.into()],
        supporting_task_id: String::new(),
        supporting_text: String::new(),
    };
    let skeleton_digest = fidelity_cluster_digest(&[], &[], &skeleton);
    let question = Question {
        obligation: &obligation,
        skeleton: &skeleton_digest,
        tasks: &tasks,
        texts: &texts,
    };
    store.save(question, &refuted).unwrap();

    critic.reset();
    let evaluation = evaluate(cwd, Ok(critic.clone())).await;
    assert!(
        critic.asked().is_empty(),
        "a contradiction is not settled by asking"
    );
    let finding = evaluation
        .findings
        .iter()
        .find(|finding| finding.text.starts_with("obligation AC-WS-001"))
        .expect("the contradiction blocks");
    assert!(
        finding.text.contains("contradict")
            && finding
                .text
                .contains(&format!("task TASK-WS-001: \"{LOOPHOLE}\""))
            && finding
                .text
                .contains(&format!("task TASK-WS-001: \"{SUPPORT}\"")),
        "{}",
        finding.text
    );
    assert_eq!(finding.subject, "AC-WS-001");
    assert!(evaluation.report.contains("G-WS-001: necessarily true"));
}

/// A true verdict whose quote is not verbatim is refused and asked again;
/// the critic is told which key to fix.
#[tokio::test]
async fn a_true_verdict_with_a_paraphrased_quote_is_asked_again_never_accepted() {
    let temp = corpus();
    let critic = FakeCritic::new(vec![
        Ok(reply(&[]).replace(SUPPORT, "cargo tests the widgets")),
        Ok(reply(&[])),
    ]);
    let evaluation = evaluate(temp.path(), Ok(critic.clone())).await;
    assert!(evaluation.operational_error().is_none());
    assert_eq!(critic.calls(), 2);
    let conversation = critic.conversations.lock().unwrap()[1].clone();
    let feedback = conversation[2]["content"].as_str().unwrap();
    assert!(
        feedback.contains("`supporting_text`") && feedback.contains("does not appear verbatim"),
        "{feedback}"
    );
}

/// A quote too short to be an anchor (here a heading's words) is refused
/// naming the rule, and the critic's next reply is taken. Each anchor rule
/// is checked one by one in `fidelity_audit_support_tests.rs`.
#[tokio::test]
async fn a_true_verdict_quoting_a_short_fragment_is_asked_again() {
    let temp = corpus();
    let critic = FakeCritic::new(vec![
        Ok(reply(&[]).replace(SUPPORT, "TASK-WS-001 — Ingest")),
        Ok(reply(&[])),
    ]);
    let evaluation = evaluate(temp.path(), Ok(critic.clone())).await;
    assert!(evaluation.operational_error().is_none());
    assert_eq!(critic.calls(), 2);
    let conversation = critic.conversations.lock().unwrap()[1].clone();
    let feedback = conversation[2]["content"].as_str().unwrap();
    assert!(
        feedback.contains("`supporting_text`") && feedback.contains("it needs at least 8 words"),
        "{feedback}"
    );
}
