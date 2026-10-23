//! C1: the critic reads a reply's finish reason before parsing it, by the
//! acceptance judge's rule. A reply cut at the output limit is continued
//! while its JSON document is open; one that never completes is the
//! critic's unusable reply (an authored judge reply), never operational.
use super::*;
use archon_workflow::fidelity_audit::ClaimingTask;
use async_trait::async_trait;
use std::sync::Mutex;

/// Answers each call with the next scripted `(content, stop_reason)`,
/// recording every request's messages.
struct Scripted {
    replies: Mutex<Vec<(String, &'static str)>>,
    asked: Mutex<Vec<Vec<serde_json::Value>>>,
}

#[async_trait]
impl WorkflowLlmClient for Scripted {
    async fn send_message(
        &self,
        _messages: Vec<serde_json::Value>,
        _system: Vec<serde_json::Value>,
        _tools: Vec<serde_json::Value>,
        _model: &str,
    ) -> archon_workflow::WorkflowResult<WorkflowAgentOutcome> {
        unreachable!("the critic asks with explicit sampling")
    }

    async fn send_message_with_temperature(
        &self,
        messages: Vec<serde_json::Value>,
        _system: Vec<serde_json::Value>,
        _tools: Vec<serde_json::Value>,
        _model: &str,
        _temperature: f64,
    ) -> archon_workflow::WorkflowResult<WorkflowAgentOutcome> {
        self.asked.lock().unwrap().push(messages);
        let (content, stop) = self.replies.lock().unwrap().remove(0);
        Ok(WorkflowAgentOutcome {
            content,
            stop_reason: Some(stop.into()),
            ..WorkflowAgentOutcome::default()
        })
    }
}

fn scripted(replies: Vec<(String, &'static str)>) -> Scripted {
    Scripted {
        replies: Mutex::new(replies),
        asked: Mutex::new(Vec::new()),
    }
}

const TASK: &str = "The task stores the required behavior in the shared registry on every run.";

async fn ask_once(client: &Scripted) -> (Result<Asked>, usize) {
    let obligations = [ClaimedObligation {
        id: "REQ-X-001".into(),
        text: "required behavior".into(),
    }];
    let tasks = [ClaimingTask {
        task_id: "TASK-X-001".into(),
        text: TASK.into(),
    }];
    let cache = tempfile::tempdir().unwrap();
    let mut kept = 0;
    let asked = ask(
        client,
        cache.path(),
        "digest",
        &obligations,
        &tasks,
        &SkeletonSummary::absent(),
        &FreezeBudget::unlimited(),
        &mut |_, _| {
            kept += 1;
            Ok(())
        },
    )
    .await;
    (asked, kept)
}

fn document() -> String {
    serde_json::json!({"verdicts": [{"obligation_id": "REQ-X-001", "necessarily_true": true, "weakest_task_id": "", "reason": "the task obliges it", "quoted_task_text": "", "supporting_task_id": "TASK-X-001", "supporting_text": TASK}]}).to_string()
}

#[tokio::test]
async fn a_reply_cut_at_the_output_limit_is_continued_and_then_judged() {
    let whole = document();
    let (head, tail) = whole.split_at(whole.len() / 2);
    let client = scripted(vec![
        (head.to_string(), "max_tokens"),
        (tail.to_string(), "end_turn"),
    ]);
    let (asked, kept) = ask_once(&client).await;
    assert!(
        matches!(asked, Ok(Asked::Answered)),
        "continued, then judged: {:?}",
        asked.err()
    );
    assert_eq!(kept, 1);
    let asked = client.asked.lock().unwrap();
    assert_eq!(asked.len(), 2, "one continuation");
    let roles: Vec<_> = asked[1].iter().map(|m| m["role"].as_str()).collect();
    assert_eq!(roles, [Some("user"), Some("assistant"), Some("user")]);
    assert_eq!(asked[1][1]["content"], head);
}

#[tokio::test]
async fn a_truncation_that_never_completes_is_an_unusable_critic_reply_not_operational() {
    let whole = document();
    let head = &whole[..whole.len() / 2];
    // The continuation adds nothing: no progress, so no verdict ever.
    let client = scripted(vec![
        (head.to_string(), "length"),
        (String::new(), "length"),
    ]);
    let (asked, kept) = ask_once(&client).await;
    let error = match asked {
        Err(error) => error,
        Ok(_) => panic!("a reply that never completes gives no verdict"),
    };
    assert_eq!(kept, 0);
    let envelope = operational(GateEvaluation::new("", Vec::new()), &error)
        .into_envelope()
        .unwrap();
    let operational = envelope.operational_error.expect("no usable verdict");
    assert_eq!(operational.kind, "authored_judge_reply", "{error:#}");
}
