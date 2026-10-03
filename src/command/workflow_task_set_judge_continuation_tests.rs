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

const HEAD: &str = r#"{"decisions":[{"id":"AC-X-001","verdict":"acc"#;
const TAIL: &str = r#"epted","counterexample":"none","reason":"ok"}]}"#;

/// One scripted reply: its content and finish reason, or a provider error.
type Reply = Result<(String, Option<&'static str>), String>;

/// Answers each call with the next scripted reply and keeps every request.
struct Scripted {
    replies: Mutex<VecDeque<Reply>>,
    seen: Mutex<Vec<Vec<serde_json::Value>>>,
}

impl Scripted {
    fn new(replies: Vec<Result<(&str, Option<&'static str>), &str>>) -> Self {
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

    fn calls(&self) -> Vec<Vec<serde_json::Value>> {
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
    assert!(
        incomplete.to_string().contains("added nothing"),
        "{error:#}"
    );
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
