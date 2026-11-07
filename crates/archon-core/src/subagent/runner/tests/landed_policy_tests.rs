use super::*;
use archon_tools::audit_landing::EmptyReplyPolicy;

struct PolicyHost {
    pointer: &'static str,
    records: Mutex<Vec<Value>>,
    retained_after: Option<usize>,
}
impl LandingHost for PolicyHost {
    fn accepts_record_arrays(&self) -> bool {
        false
    }
    fn empty_reply_policy(&self) -> archon_tools::audit_landing::EmptyReplyPolicy {
        archon_tools::audit_landing::EmptyReplyPolicy::Resend
    }
    fn normalize_record_reply(&self) -> bool {
        true
    }
    fn schema(&self) -> Option<serde_json::Value> {
        None
    }
    fn tool_name(&self) -> &'static str {
        "opaque-evidence-capability"
    }
    fn receipt_envelope_pointer(&self) -> &'static str {
        self.pointer
    }
    fn progress_key_fields(&self) -> &'static [&'static str] {
        &["unit"]
    }
    fn allows_unreceipted_reply(&self) -> bool {
        false
    }
    fn land(&self, value: Value) -> Result<String, String> {
        self.records.lock().unwrap().push(value);
        Ok("retained".into())
    }
    fn hint(&self) -> Result<String, String> {
        Ok("Send the receipt".into())
    }
    fn complete(&self, value: &Value) -> Result<(), String> {
        if value["records_landed"] == self.records.lock().unwrap().len() {
            Ok(())
        } else {
            Err("wrong count".into())
        }
    }
    fn retained_completion(&self) -> Option<Value> {
        let count = self.records.lock().unwrap().len();
        self.retained_after
            .filter(|minimum| count >= *minimum)
            .map(|_| json!({"records_landed":count}))
    }
}
fn policy_runner(provider: Arc<MockProvider>, host: Arc<PolicyHost>) -> SubagentRunner {
    let mut runner = make_runner(provider, 20);
    runner.tool_context.audit_landing = Some(Arc::new(AuditLanding::new(host, None)));
    runner
}
fn policy_host(pointer: &'static str) -> Arc<PolicyHost> {
    Arc::new(PolicyHost {
        pointer,
        records: Mutex::new(vec![]),
        retained_after: None,
    })
}
fn empty_end_turn() -> Vec<StreamEvent> {
    vec![
        StreamEvent::MessageDelta {
            stop_reason: Some("end_turn".into()),
            usage: None,
        },
        StreamEvent::MessageStop,
    ]
}

#[tokio::test(start_paused = true)]
async fn landed_resend_policies_retry_empty_end_turn_before_receipt() {
    // Audit, review and verify envelopes, all using the same unrelated tool name.
    for pointer in ["/data/repository_audit", "/data", "/verification"] {
        let host = policy_host(pointer);
        assert_eq!(host.empty_reply_policy(), EmptyReplyPolicy::Resend);
        let mut reply = json!({});
        let mut current = &mut reply;
        for field in pointer.trim_start_matches('/').split('/') {
            current[field] = json!({});
            current = &mut current[field];
        }
        *current = json!({"records_landed":0});
        let receipt = reply.to_string();
        let provider = Arc::new(MockProvider::new(vec![
            empty_end_turn(),
            text_response(&receipt),
        ]));
        let runner = policy_runner(provider.clone(), host);
        assert_eq!(runner.run("inspect").await.unwrap(), receipt);
        assert_eq!(provider.requests().len(), 2);
        assert_eq!(
            provider.requests()[0].messages,
            provider.requests()[1].messages
        );
    }
}

#[tokio::test(start_paused = true)]
async fn landed_audit_empty_reply_is_never_final_empty() {
    let provider = Arc::new(MockProvider::new(vec![empty_end_turn(); 100]));
    let mut runner = policy_runner(provider.clone(), policy_host("/data/repository_audit"));
    runner.timeout_secs = 1;
    Arc::make_mut(&mut runner.agent_config).subagent_stream_idle_timeout_secs = 1;
    let error = archon_tools::provider_recovery::scope(runner.run("inspect"))
        .await
        .unwrap_err();
    assert!(
        error
            .to_string()
            .contains(archon_llm::transport_idle::TRANSPORT_STALL_MARKER),
        "{error}"
    );
    assert!(provider.requests().len() > 3);
}

#[tokio::test(start_paused = true)]
async fn landed_incomplete_empty_exhaustion_can_use_retained_completion() {
    let host = Arc::new(PolicyHost {
        retained_after: Some(1),
        ..PolicyHost {
            pointer: "",
            records: Mutex::new(vec![json!({"unit":"one"})]),
            retained_after: None,
        }
    });
    let provider = Arc::new(MockProvider::new(vec![empty_end_turn(); 100]));
    let mut runner = policy_runner(provider.clone(), host);
    runner.timeout_secs = 1;
    Arc::make_mut(&mut runner.agent_config).subagent_stream_idle_timeout_secs = 1;
    let completion = archon_tools::provider_recovery::scope(runner.run_with_completion("inspect"))
        .await
        .unwrap();
    assert_eq!(completion.content, r#"{"records_landed":1}"#);
    assert_eq!(completion.stop_reason.as_deref(), Some("end_turn"));
    assert!(provider.requests().len() > 3);
}

#[tokio::test]
async fn landed_duplicate_record_replies_spend_only_the_no_progress_budget() {
    let host = policy_host("/data");
    let provider = Arc::new(MockProvider::new(vec![
        text_response(r#"{"unit":"one"}"#);
        10
    ]));
    let runner = policy_runner(provider.clone(), host.clone());
    let error = runner.run("inspect").await.unwrap_err();
    assert!(
        error
            .downcast_ref::<archon_tools::audit_landing::IncompleteLandedArtifact>()
            .is_some(),
        "{error}"
    );
    assert_eq!(
        provider.requests().len(),
        4,
        "one new record followed by three unchanged replies"
    );
    assert_eq!(host.records.lock().unwrap().len(), 4);
}

#[tokio::test]
async fn landed_changed_record_replies_exhaust_the_receipt_budget_and_keep_landings() {
    let host = policy_host("/data");
    let replies = (0..10)
        .map(|revision| {
            text_response(
                &json!({"unit":"one", "summary":format!("revision {revision}")}).to_string(),
            )
        })
        .collect();
    let provider = Arc::new(MockProvider::new(replies));
    let runner = policy_runner(provider.clone(), host.clone());
    let error = runner.run("inspect").await.unwrap_err();
    assert!(
        error
            .downcast_ref::<archon_tools::audit_landing::IncompleteLandedArtifact>()
            .is_some(),
        "{error}"
    );
    assert_eq!(
        provider.requests().len(),
        4,
        "first key plus three receipt retries"
    );
    let records = host.records.lock().unwrap();
    assert_eq!(records.len(), 4, "all accepted landings remain retained");
    assert_eq!(records.last().unwrap()["summary"], "revision 3");
}

#[tokio::test]
async fn landed_array_advice_requires_array_capability() {
    let host = policy_host("/data");
    let provider = Arc::new(MockProvider::new(vec![
        text_response(r#"{"unit":"one"} then {"unit":"two"}"#),
        text_response(r#"{"data":{"records_landed":0}}"#),
    ]));
    let runner = policy_runner(provider.clone(), host);
    assert!(runner.run("inspect").await.is_ok());
    let feedback = serde_json::to_string(&provider.requests()[1].messages).unwrap();
    assert!(feedback.contains("send one record object"));
    assert!(!feedback.contains("reply that is only a JSON array"));
}

#[tokio::test]
async fn landed_reworded_seed_subject_stops_at_receipt_budget_with_latest_record_retained() {
    let host = Arc::new(Host::default());
    let seed: Value = serde_json::from_str(&record("TASK-X-001")).unwrap();
    host.records.lock().unwrap().push(seed.clone());
    let replies = (0..10)
        .map(|revision| {
            let mut value = seed.clone();
            value["summary"] = json!(format!("inspection revision {revision}"));
            text_response(&value.to_string())
        })
        .collect();
    let provider = Arc::new(MockProvider::new(replies));
    let runner = runner_with(provider.clone(), host.clone(), 20);
    let error = runner.run("inspect").await.unwrap_err();
    assert!(
        error
            .downcast_ref::<archon_tools::audit_landing::IncompleteLandedArtifact>()
            .is_some(),
        "{error}"
    );
    assert_eq!(provider.requests().len(), 4);
    assert_eq!(host.count(), 1);
    assert_eq!(
        host.records.lock().unwrap()[0]["summary"],
        "inspection revision 3"
    );
    assert_eq!(
        host.new_count(),
        0,
        "rewording metadata never changes the retained task"
    );
}
