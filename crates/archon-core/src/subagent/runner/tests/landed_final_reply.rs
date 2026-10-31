//! Issue-550: the final reply of a session that lands records. A record given
//! as the final reply lands like any other, and a session that would fail
//! while the host can complete it from retained records ends with that
//! completion instead of "incomplete landed artifact" or a no-progress stop.
use super::*;
use archon_tools::audit_landing::{AuditLanding, LandingHost};
use serde_json::{Value, json};

/// Rounds of identical tool results that end a write-capable session.
const ROUNDS_TO_STOP: usize = 16;

#[derive(Default)]
struct Host {
    records: Mutex<Vec<Value>>,
}
impl Host {
    fn count(&self) -> usize {
        self.records.lock().unwrap().len()
    }
}
impl LandingHost for Host {
    fn tool_name(&self) -> &'static str {
        "land-skeleton-record"
    }
    fn land(&self, value: Value) -> Result<String, String> {
        if value.get("task").is_none() {
            return Err("skeleton record requires task".into());
        }
        self.records.lock().unwrap().push(value);
        self.hint()
    }
    fn hint(&self) -> Result<String, String> {
        Ok(format!(
            "Host retained {0} records. Final reply: {{\"records_landed\":{0}}}",
            self.count()
        ))
    }
    fn complete(&self, value: &Value) -> Result<(), String> {
        match value["records_landed"].as_u64() {
            Some(n) if n as usize == self.count() => Ok(()),
            _ => Err(format!(
                "records_landed=missing but host retained {} record(s)",
                self.count()
            )),
        }
    }
    fn retained_completion(&self) -> Option<Value> {
        (self.count() > 0).then(|| json!({"records_landed": self.count()}))
    }
}

fn runner_with(provider: Arc<MockProvider>, host: Arc<Host>, max_turns: u32) -> SubagentRunner {
    let mut runner = make_runner(provider, max_turns);
    runner.tool_context.audit_landing = Some(Arc::new(AuditLanding::new(host, None)));
    runner
}

fn record(id: &str) -> String {
    json!({"subject":id,"summary":"unit","evidence":[{"kind":"inspection","summary":"read"}],
        "task":{"task_id":id,"file_name":format!("{id}.md"),"implements":["REQ-1"]}})
    .to_string()
}

#[tokio::test]
async fn issue550_a_final_reply_that_is_one_record_lands_and_the_session_continues() {
    let host = Arc::new(Host::default());
    let provider = Arc::new(MockProvider::new(vec![
        text_response(&record("TASK-X-001")),
        text_response(r#"{"records_landed":1}"#),
    ]));
    let runner = runner_with(provider.clone(), host.clone(), 4);

    let answer = runner.run("author the skeleton").await.unwrap();

    assert_eq!(answer, r#"{"records_landed":1}"#);
    assert_eq!(host.count(), 1, "the record reply was landed");
    let requests = provider.requests();
    assert_eq!(requests.len(), 2);
    let asked = serde_json::to_string(&requests[1].messages).unwrap();
    assert!(asked.contains("not the final reply"), "{asked}");
    assert!(asked.contains(r#"{\"records_landed\":1}"#), "{asked}");
}

#[tokio::test]
async fn issue550_a_record_reply_the_host_cannot_land_is_asked_again_with_the_error() {
    let host = Arc::new(Host::default());
    let provider = Arc::new(MockProvider::new(vec![
        text_response(r#"{"subject":"TASK-X-001","summary":"no task"}"#),
        text_response(&record("TASK-X-001")),
        text_response(r#"{"records_landed":1}"#),
    ]));
    let runner = runner_with(provider.clone(), host.clone(), 4);

    assert_eq!(
        runner.run("author the skeleton").await.unwrap(),
        r#"{"records_landed":1}"#
    );
    let asked = serde_json::to_string(&provider.requests()[1].messages).unwrap();
    assert!(asked.contains("skeleton record requires task"), "{asked}");
}

#[tokio::test]
async fn issue550_repeated_incomplete_replies_complete_from_retained_records() {
    let host = Arc::new(Host::default());
    for id in ["TASK-X-001", "TASK-X-002"] {
        host.land(serde_json::from_str(&record(id)).unwrap())
            .unwrap();
    }
    // The live shape: the count given as the list of landed subjects.
    let wrong = r#"{"schema_version":1,"acceptance_digest":"","records_landed":["TASK-X-001","TASK-X-002"]}"#;
    let provider = Arc::new(MockProvider::new(vec![
        text_response(wrong),
        text_response(wrong),
        text_response(wrong),
    ]));
    let runner = runner_with(provider.clone(), host, 5);

    let answer = runner
        .run("author the skeleton")
        .await
        .expect("completed from the retained records, not failed");

    assert_eq!(answer, r#"{"records_landed":2}"#);
    assert_eq!(provider.requests().len(), 3);
}

#[tokio::test]
async fn issue550_without_retained_records_repeated_incomplete_replies_still_fail() {
    let provider = Arc::new(MockProvider::new(vec![
        text_response(r#"{"records_landed":3}"#),
        text_response(r#"{"records_landed":3}"#),
        text_response(r#"{"records_landed":3}"#),
    ]));
    let runner = runner_with(provider, Arc::new(Host::default()), 5);

    let error = runner.run("author").await.unwrap_err().to_string();

    assert!(error.contains("incomplete landed artifact"), "{error}");
}

#[tokio::test]
async fn issue550_a_loop_guard_stop_completes_from_retained_records() {
    let dir = tempfile::tempdir().unwrap();
    let host = Arc::new(Host::default());
    host.land(serde_json::from_str(&record("TASK-X-001")).unwrap())
        .unwrap();
    let input = json!({ "file_path": dir.path().join("missing.json") }).to_string();
    let mut responses: Vec<_> = (0..ROUNDS_TO_STOP)
        .map(|index| tool_use_response(&format!("read-{index}"), "Read", &input))
        .collect();
    responses.push(text_response("never asked"));
    let provider = Arc::new(MockProvider::new(responses));
    let mut runner = runner_with(provider.clone(), host, 40);
    runner.tool_context.working_dir = dir.path().to_path_buf();
    runner.tool_context.session_id = "landed-loop-guard".into();
    runner.tool_context.subagent_id = Some("landed-loop-guard-agent".into());
    runner.tool_context.workflow_read_guard = Some(Arc::new(
        archon_tools::workflow_read_guard::WorkflowReadGuard::new(1_000, 1_000, false, false),
    ));

    let completion = runner
        .run_with_completion("author the skeleton")
        .await
        .expect("the stop completes from the retained record");

    assert_eq!(completion.content, r#"{"records_landed":1}"#);
    assert_eq!(completion.stop_reason.as_deref(), Some("end_turn"));
    assert_eq!(provider.requests().len(), ROUNDS_TO_STOP);
}
