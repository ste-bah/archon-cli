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
    baseline: Mutex<Option<Vec<Value>>>,
}
impl Host {
    fn count(&self) -> usize {
        self.records.lock().unwrap().len()
    }
    fn new_count(&self) -> usize {
        let baseline = self.baseline.lock().unwrap();
        let Some(before) = baseline.as_ref() else {
            return 0;
        };
        let records = self.records.lock().unwrap();
        let subjects: std::collections::BTreeSet<_> = before
            .iter()
            .chain(records.iter())
            .map(|v| v["subject"].as_str().unwrap())
            .collect();
        subjects
            .into_iter()
            .filter(|subject| {
                let task = |rows: &[Value]| {
                    rows.iter()
                        .find(|v| v["subject"] == *subject)
                        .map(|v| v["task"].clone())
                };
                task(before) != task(&records)
            })
            .count()
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
        {
            let mut baseline = self.baseline.lock().unwrap();
            let mut records = self.records.lock().unwrap();
            baseline.get_or_insert_with(|| records.clone());
            if let Some(record) = records
                .iter_mut()
                .find(|v| v["subject"] == value["subject"])
            {
                *record = value;
            } else {
                records.push(value);
            }
        }
        self.hint()
    }
    fn hint(&self) -> Result<String, String> {
        Ok(format!(
            "Host retained {} records; {} new landings in this call (seeds do not count). When no task is left to land, your final reply is exactly {{\"records_landed\":{}}}",
            self.count(),
            self.new_count(),
            self.new_count()
        ))
    }
    fn complete(&self, value: &Value) -> Result<(), String> {
        match value["records_landed"].as_u64() {
            Some(n)
                if n as usize == self.new_count()
                    && value.as_object().is_some_and(|v| v.len() == 1) =>
            {
                Ok(())
            }
            _ => Err(format!(
                "reply must count {} new landings in this call, excluding seeds",
                self.new_count()
            )),
        }
    }
    fn retained_completion(&self) -> Option<Value> {
        (self.new_count() > 0).then(|| json!({"records_landed": self.new_count()}))
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

    let error = runner.run("author").await.unwrap_err();
    assert!(
        error
            .downcast_ref::<archon_tools::audit_landing::IncompleteLandedArtifact>()
            .is_some()
    );
    let error = error.to_string();

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

#[tokio::test]
async fn issue550_a_host_completion_is_a_whole_reply_whatever_the_cut_off_stop_reason() {
    let host = Arc::new(Host::default());
    host.land(serde_json::from_str(&record("TASK-X-001")).unwrap())
        .unwrap();
    let wrong = r#"{"records_landed":9}"#;
    // The third incomplete reply arrives cut off by the provider's budget.
    let mut cut_off = text_response(wrong);
    let stop = cut_off.len() - 1;
    cut_off.insert(
        stop,
        StreamEvent::MessageDelta {
            stop_reason: Some("max_tokens".into()),
            usage: None,
        },
    );
    let provider = Arc::new(MockProvider::new(vec![
        text_response(wrong),
        text_response(wrong),
        cut_off,
    ]));
    let runner = runner_with(provider.clone(), host, 5);

    let completion = runner
        .run_with_completion("author the skeleton")
        .await
        .expect("completed from the retained record");

    assert_eq!(completion.content, r#"{"records_landed":1}"#);
    assert_eq!(completion.stop_reason.as_deref(), Some("end_turn"));
}

#[tokio::test]
async fn landed_review_verify_replies_without_receipts_are_refused() {
    struct OtherHost(&'static str);
    impl LandingHost for OtherHost {
        fn tool_name(&self) -> &'static str {
            self.0
        }
        fn land(&self, _: Value) -> Result<String, String> {
            panic!("no landing")
        }
        fn hint(&self) -> Result<String, String> {
            Ok(String::new())
        }
        fn complete(&self, _: &Value) -> Result<(), String> {
            panic!("no receipt")
        }
    }
    for kind in [
        "land-review-record",
        "land-verify-record",
        "land-audit-record",
    ] {
        for text in ["Review complete.", r#"{"status":"accepted","data":{}}"#] {
            let provider = Arc::new(MockProvider::new(vec![text_response(text); 3]));
            let mut runner = make_runner(provider.clone(), 4);
            runner.tool_context.audit_landing =
                Some(Arc::new(AuditLanding::new(Arc::new(OtherHost(kind)), None)));
            if kind == "land-audit-record" {
                assert_eq!(runner.run("Inspect source").await.unwrap(), text);
                assert_eq!(provider.requests().len(), 1);
                continue;
            }
            // A-runner.md D6 / coverage line 210 asserted the defect for review/verify.
            let error = runner.run("Inspect source").await.unwrap_err();
            assert!(
                error
                    .downcast_ref::<archon_tools::audit_landing::IncompleteLandedArtifact>()
                    .is_some()
            );
            assert_eq!(provider.requests().len(), 3);
        }
    }
}

#[tokio::test]
async fn landed_non_skeleton_good_record_replies_continue_to_the_receipt() {
    struct ReviewHost(Mutex<Vec<Value>>);
    impl LandingHost for ReviewHost {
        fn tool_name(&self) -> &'static str {
            "land-review-record"
        }
        fn land(&self, record: Value) -> Result<String, String> {
            self.0.lock().unwrap().push(record);
            Ok("Continue with the final verdict".into())
        }
        fn hint(&self) -> Result<String, String> {
            Ok(String::new())
        }
        fn complete(&self, _: &Value) -> Result<(), String> {
            Ok(())
        }
    }
    let host = Arc::new(ReviewHost(Mutex::new(vec![])));
    let mut responses: Vec<_> = (1..=4)
        .map(|n| text_response(&json!({"subject":format!("UNIT-{n}")}).to_string()))
        .collect();
    responses.push(text_response(r#"{"data":{"records_landed":4}}"#));
    let provider = Arc::new(MockProvider::new(responses));
    let mut runner = make_runner(provider.clone(), 8);
    runner.tool_context.audit_landing = Some(Arc::new(AuditLanding::new(host.clone(), None)));
    assert_eq!(
        runner.run("Review source").await.unwrap(),
        r#"{"data":{"records_landed":4}}"#
    );
    assert_eq!(host.0.lock().unwrap().len(), 4);
    assert_eq!(provider.requests().len(), 5);
}

// D2, D-doubles.md §4.1: retained seeds and identical re-landings are not new work.
#[test]
fn landed_fake_host_counts_only_this_calls_byte_changing_subjects() {
    let seed: Value = serde_json::from_str(&record("TASK-X-001")).unwrap();
    let host = Host::default();
    host.records.lock().unwrap().push(seed.clone());
    assert!(
        host.retained_completion().is_none(),
        "seed-only store cannot complete"
    );
    host.land(seed.clone()).unwrap();
    assert!(
        host.retained_completion().is_none(),
        "identical seed is not new work"
    );
    let mut changed = seed.clone();
    changed["task"]["implements"] = json!(["REQ-2"]);
    host.land(changed).unwrap();
    assert_eq!(
        host.retained_completion(),
        Some(json!({"records_landed":1}))
    );
    host.land(seed).unwrap();
    assert!(
        host.retained_completion().is_none(),
        "restoring seed bytes is not new work"
    );
    let mut metadata = serde_json::from_str::<Value>(&record("TASK-X-001")).unwrap();
    metadata["summary"] = json!("same task, different inspection summary");
    host.land(metadata).unwrap();
    assert!(
        host.retained_completion().is_none(),
        "only task bytes count"
    );
    let next: Value = serde_json::from_str(&record("TASK-X-002")).unwrap();
    host.land(next.clone()).unwrap();
    host.land(next).unwrap();
    assert_eq!(host.count(), 2, "same subject replaces rather than appends");
    assert_eq!(
        host.retained_completion(),
        Some(json!({"records_landed":1}))
    );
    assert!(host.complete(&json!({"records_landed":1})).is_ok());
    assert!(host.complete(&json!({"records_landed":2})).is_err());
    assert!(
        host.complete(&json!({"records_landed":1,"extra":true}))
            .is_err()
    );
    assert!(host.hint().unwrap().contains("your final reply is exactly"));
}

#[path = "landing_audit_tests.rs"]
mod landing_audit_tests;
