//! Issue 260: a truncated judge reply is continued, never accepted and never
//! the end of the run; a judge that cannot complete is incomplete (resumable).

use std::collections::{BTreeSet, VecDeque};
use std::sync::Mutex;

use archon_workflow::error::{WorkflowError, WorkflowResult};
use archon_workflow::llm_client_port::WorkflowAgentOutcome;
use archon_workflow::task_set_contract::{
    AcceptanceCheck, AcceptanceContract, AcceptanceCriterion, GapPolicy, JudgeDecision,
    JudgeVerdict, PrdIdentity, TrustedCwd,
};
use async_trait::async_trait;

use super::*;

fn contract() -> AcceptanceContract {
    AcceptanceContract {
        schema_version: 1,
        prd: PrdIdentity {
            path: "p".into(),
            digest: "d".into(),
        },
        gap_policy: GapPolicy {
            permitted_acceptance_ids: Default::default(),
            forbidden_phrases: Vec::new(),
            required_fields: Vec::new(),
        },
        acceptance: vec![AcceptanceCriterion {
            id: "AC-X-001".into(),
            criterion: "c".into(),
            check: AcceptanceCheck::Command {
                command: "true".into(),
                cwd: TrustedCwd::ProjectRoot,
            },
            gap_permitted: false,
            covers: Vec::new(),
            judgment: JudgeVerdict {
                verdict: JudgeDecision::Refuted,
                counterexample: String::new(),
                reason: String::new(),
                host_call_id: String::new(),
                sampling: None,
            },
        }],
        supplementary: Vec::new(),
    }
}

pub(crate) const HEAD: &str = r#"{"decisions":[{"id":"AC-X-001","verdict":"acc"#;
pub(crate) const TAIL: &str = r#"epted","counterexample":"none","reason":"ok"}]}"#;
const ACCEPTED: &str = r#"{"decisions":[{"id":"AC-X-001","verdict":"accepted","counterexample":"none","reason":"ok"}]}"#;
const REFUTED: &str = r#"{"decisions":[{"id":"AC-X-001","verdict":"refuted","counterexample":"a stub","reason":"it passes on a stub"}]}"#;

/// One scripted reply: its content and finish reason, or a provider error.
type Reply = Result<(String, Option<&'static str>), String>;

/// Answers each call with the next scripted reply and keeps every request.
pub(crate) struct Scripted {
    replies: Mutex<VecDeque<Reply>>,
    seen: Mutex<Vec<Vec<serde_json::Value>>>,
}

impl Scripted {
    pub(crate) fn new(replies: Vec<Result<(&str, Option<&'static str>), &str>>) -> Self {
        Self {
            replies: Mutex::new(
                replies
                    .into_iter()
                    .map(|reply| {
                        reply
                            .map(|(text, stop)| (text.to_string(), stop))
                            .map_err(str::to_string)
                    })
                    .collect(),
            ),
            seen: Mutex::new(Vec::new()),
        }
    }

    pub(crate) fn calls(&self) -> Vec<Vec<serde_json::Value>> {
        self.seen.lock().unwrap().clone()
    }
}

#[async_trait]
impl WorkflowLlmClient for Scripted {
    async fn send_message_with_temperature(
        &self,
        messages: Vec<serde_json::Value>,
        _system: Vec<serde_json::Value>,
        _tools: Vec<serde_json::Value>,
        _model: &str,
        temperature: f64,
    ) -> WorkflowResult<WorkflowAgentOutcome> {
        assert_eq!(temperature, 0.0);
        self.seen.lock().unwrap().push(messages);
        let reply = self.replies.lock().unwrap().pop_front();
        match reply.expect("no scripted reply left") {
            Ok((content, stop)) => Ok(WorkflowAgentOutcome {
                content,
                stop_reason: stop.map(str::to_string),
                ..WorkflowAgentOutcome::default()
            }),
            Err(message) => Err(WorkflowError::port(std::io::Error::other(message))),
        }
    }

    async fn send_message(
        &self,
        _messages: Vec<serde_json::Value>,
        _system: Vec<serde_json::Value>,
        _tools: Vec<serde_json::Value>,
        _model: &str,
    ) -> WorkflowResult<WorkflowAgentOutcome> {
        unreachable!("the judge always samples explicitly")
    }
}

fn expected() -> BTreeSet<String> {
    BTreeSet::from(["AC-X-001".to_string()])
}

#[tokio::test]
async fn a_truncated_reply_is_continued_and_the_whole_reply_is_judged() {
    let client = Scripted::new(vec![
        Ok((HEAD, Some("max_tokens"))),
        Ok((TAIL, Some("end_turn"))),
    ]);

    let judged = judge_contract(&client, contract(), &expected())
        .await
        .expect("a continued reply is a complete reply");

    assert_eq!(
        judged.acceptance[0].judgment.verdict,
        JudgeDecision::Accepted
    );
    let calls = client.calls();
    assert_eq!(calls.len(), 2, "one continuation, no re-ask");
    // The continuation carries the task, the partial reply, and the ask.
    assert_eq!(calls[1].len(), 3);
    assert_eq!(calls[1][0], calls[0][0]);
    assert_eq!(calls[1][1]["role"], "assistant");
    assert_eq!(calls[1][1]["content"], HEAD);
    assert_eq!(calls[1][2]["role"], "user");
}

#[tokio::test]
async fn a_continuation_that_adds_nothing_is_incomplete_never_a_verdict() {
    let client = Scripted::new(vec![
        Ok((HEAD, Some("max_tokens"))),
        Ok(("", Some("length"))),
    ]);

    let error = judge_contract(&client, contract(), &expected())
        .await
        .expect_err("a truncated verdict is never accepted");

    let incomplete = JudgeIncomplete::caused(&error).expect("incomplete, not a failure");
    assert!(incomplete.to_string().contains("no token"), "{error:#}");
    assert_eq!(client.calls().len(), 2);
}

#[tokio::test]
async fn a_continuation_that_repeats_itself_is_incomplete() {
    let client = Scripted::new(vec![
        Ok((HEAD, Some("max_tokens"))),
        Ok((HEAD, Some("max_tokens"))),
        Ok((HEAD, Some("max_tokens"))),
    ]);

    let error = judge_contract(&client, contract(), &expected())
        .await
        .expect_err("repeating the same text is no progress");

    assert!(JudgeIncomplete::caused(&error).is_some(), "{error:#}");
    assert_eq!(client.calls().len(), 2);
}

#[tokio::test]
async fn a_provider_error_is_incomplete_not_a_failure() {
    let client = Scripted::new(vec![Err("upstream 503")]);

    let error = judge_contract(&client, contract(), &expected())
        .await
        .expect_err("no reply");

    let incomplete = JudgeIncomplete::caused(&error).expect("incomplete");
    assert!(incomplete.to_string().contains("upstream 503"), "{error:#}");
}

#[tokio::test]
async fn replies_that_never_parse_end_incomplete_after_the_no_progress_bound() {
    let client = Scripted::new(vec![
        Ok(("no json", Some("end_turn"))),
        Ok(("still none", Some("end_turn"))),
        Ok(("nothing", Some("end_turn"))),
    ]);

    let error = judge_contract(&client, contract(), &expected())
        .await
        .expect_err("no usable verdict");

    assert!(JudgeIncomplete::caused(&error).is_some(), "{error:#}");
    assert_eq!(client.calls().len(), JUDGE_ATTEMPTS);
}

/// Round 2 (P1): a truncated reply that already holds a complete document is
/// not continued (a continuation restarting with another document would be
/// read as the first), and it is never accepted: the judge is asked afresh.
#[tokio::test]
async fn a_truncated_reply_holding_a_whole_document_is_re_asked_never_continued() {
    let client = Scripted::new(vec![
        Ok((ACCEPTED, Some("max_tokens"))),
        Ok((REFUTED, Some("end_turn"))),
    ]);

    let judged = judge_contract(&client, contract(), &expected())
        .await
        .expect("the fresh reply is complete");

    assert_eq!(
        judged.acceptance[0].judgment.verdict,
        JudgeDecision::Refuted
    );
    let calls = client.calls();
    assert_eq!(calls.len(), 2);
    assert_eq!(calls[1].len(), 1, "a fresh ask, not a continuation");
}

/// Round 2 (P1): a continuation that restarts the document contradicts the
/// text it must extend: no progress, so the judge is incomplete.
#[tokio::test]
async fn a_continuation_that_restarts_the_document_is_incomplete() {
    let client = Scripted::new(vec![
        Ok((HEAD, Some("max_tokens"))),
        Ok((REFUTED, Some("end_turn"))),
    ]);

    let error = judge_contract(&client, contract(), &expected())
        .await
        .expect_err("a restart is never spliced onto the partial reply");

    assert!(JudgeIncomplete::caused(&error).is_some(), "{error:#}");
    assert_eq!(client.calls().len(), 2);
}

/// Round 2 (P1): an empty continuation that ends normally does not certify
/// the truncated reply before it.
#[tokio::test]
async fn an_empty_normally_ended_continuation_is_incomplete() {
    let client = Scripted::new(vec![
        Ok((HEAD, Some("max_tokens"))),
        Ok(("", Some("end_turn"))),
    ]);

    let error = judge_contract(&client, contract(), &expected())
        .await
        .expect_err("the document is still incomplete");

    assert!(JudgeIncomplete::caused(&error).is_some(), "{error:#}");
}

/// Round 2 (P2): progress is the document growing, never chunk inequality:
/// a long string legitimately repeats text.
#[tokio::test]
async fn identical_chunks_that_extend_the_document_are_progress() {
    let head = r#"{"decisions":[{"id":"AC-X-001","verdict":"accepted","counterexample":"none","reason":"ab"#;
    let client = Scripted::new(vec![
        Ok((head, Some("max_tokens"))),
        Ok(("ab", Some("max_tokens"))),
        Ok(("ab", Some("max_tokens"))),
        Ok((r#""}]}"#, Some("end_turn"))),
    ]);

    let judged = judge_contract(&client, contract(), &expected())
        .await
        .expect("each chunk extended the document");

    assert_eq!(judged.acceptance[0].judgment.reason, "ababab");
}

/// Round 2 (P1): no total count of continuations while each one extends.
#[tokio::test]
async fn extending_continuations_have_no_total_limit() {
    let head =
        r#"{"decisions":[{"id":"AC-X-001","verdict":"accepted","counterexample":"none","reason":""#;
    let chunks: Vec<String> = (0..80).map(|n| format!("w{n} ")).collect();
    let mut replies = vec![Ok((head, Some("max_tokens")))];
    replies.extend(
        chunks
            .iter()
            .map(|chunk| Ok((chunk.as_str(), Some("max_tokens")))),
    );
    replies.push(Ok((r#""}]}"#, Some("end_turn"))));
    let client = Scripted::new(replies);

    let judged = judge_contract(&client, contract(), &expected())
        .await
        .expect("80 extending continuations complete the reply");

    assert!(judged.acceptance[0].judgment.reason.starts_with("w0 w1 "));
    assert_eq!(client.calls().len(), 82);
}

/// Round 3 (decision D): a chunk that adds only whitespace does not move
/// the document's parse position over a token: no progress.
#[tokio::test]
async fn a_whitespace_only_continuation_is_no_progress() {
    let client = Scripted::new(vec![
        Ok((r#"{"decisions":["#, Some("max_tokens"))),
        Ok(("   ", Some("max_tokens"))),
    ]);

    let error = judge_contract(&client, contract(), &expected())
        .await
        .expect_err("whitespace never funds a continuation");

    assert!(JudgeIncomplete::caused(&error).is_some(), "{error:#}");
    assert_eq!(client.calls().len(), 2);
}

/// Round 3 (decision D): prose with no document is not continued; the
/// judge is asked afresh.
#[tokio::test]
async fn a_truncated_reply_with_no_document_is_re_asked_never_continued() {
    let client = Scripted::new(vec![
        Ok(("Let me weigh each check in turn", Some("max_tokens"))),
        Ok((ACCEPTED, Some("end_turn"))),
    ]);

    let judged = judge_contract(&client, contract(), &expected())
        .await
        .expect("the fresh reply is complete");

    assert_eq!(
        judged.acceptance[0].judgment.verdict,
        JudgeDecision::Accepted
    );
    assert_eq!(
        client.calls()[1].len(),
        1,
        "a fresh ask, not a continuation"
    );
}

/// Round 3 (decision D): a partial reply past the byte cap is no progress.
#[tokio::test]
async fn a_reply_past_the_byte_cap_is_incomplete() {
    let head =
        r#"{"decisions":[{"id":"AC-X-001","verdict":"accepted","counterexample":"none","reason":""#;
    let huge = "x".repeat(9 * 1024 * 1024);
    let client = Scripted::new(vec![
        Ok((head, Some("max_tokens"))),
        Ok((huge.as_str(), Some("max_tokens"))),
        Ok((r#""}]}"#, Some("end_turn"))),
    ]);

    let error = judge_contract(&client, contract(), &expected())
        .await
        .expect_err("no verdict needs megabytes");

    assert!(JudgeIncomplete::caused(&error).is_some(), "{error:#}");
    assert_eq!(client.calls().len(), 2);
}

#[path = "workflow_task_set_judge_idle_tests.rs"]
mod idle;

#[path = "workflow_task_set_judge_transport_tests.rs"]
mod transport;
