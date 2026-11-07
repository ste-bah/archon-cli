//! Session reply matrix, exercised with the real store, bridge and runner parser.
/*
Pure JSON receipt + new landings: compose the entire retained store.
JSON receipt + zero new landings: first draft keeps its existing behavior;
  with open findings retry within the bounded budget, including seed-only receipts.
Prose then JSON: complete from this call's retained landings, else bounded retry.
Pure prose: complete from this call's retained landings, else bounded retry.
Empty: complete from this call's retained landings, else bounded retry.
Forced answer at the tool-call ceiling: apply the same rules; seeds alone
  cannot complete a repair, and prose never becomes candidate content.
*/
use super::*;
use archon_core::{agent::AgentConfig, subagent::runner::SubagentRunner};
use archon_llm::{
    identity::{IdentityMode, IdentityProvider},
    provider::{LlmError, LlmProvider, LlmRequest, LlmResponse, ModelInfo, ProviderFeature},
    streaming::StreamEvent,
    types::{ContentBlockType, Usage},
};
use archon_tools::{audit_landing::AuditLanding, tool::ToolContext};
use std::{collections::VecDeque, sync::Mutex};

struct ReplyProvider {
    landing: Arc<AuditLanding>,
    rounds: Mutex<VecDeque<(Vec<Value>, Vec<StreamEvent>)>>,
    requests: Mutex<Vec<LlmRequest>>,
}
#[async_trait::async_trait]
impl LlmProvider for ReplyProvider {
    fn name(&self) -> &str {
        "scripted"
    }
    fn models(&self) -> Vec<ModelInfo> {
        vec![]
    }
    fn supports_feature(&self, _: ProviderFeature) -> bool {
        false
    }
    async fn complete(&self, _: LlmRequest) -> Result<LlmResponse, LlmError> {
        unreachable!()
    }
    async fn stream(
        &self,
        request: LlmRequest,
    ) -> Result<tokio::sync::mpsc::Receiver<StreamEvent>, LlmError> {
        self.requests.lock().unwrap().push(request);
        let (records, events) = self
            .rounds
            .lock()
            .unwrap()
            .pop_front()
            .expect("bounded scripted requests");
        for record in records {
            self.landing.land(record).unwrap();
        }
        let (tx, rx) = tokio::sync::mpsc::channel(16);
        tokio::spawn(async move {
            for event in events {
                tx.send(event).await.unwrap();
            }
        });
        Ok(rx)
    }
}
fn reply(text: &str) -> Vec<StreamEvent> {
    vec![
        StreamEvent::MessageStart {
            id: "reply".into(),
            model: "scripted".into(),
            usage: Usage::default(),
        },
        StreamEvent::ContentBlockStart {
            index: 0,
            block_type: ContentBlockType::Text,
            tool_use_id: None,
            tool_name: None,
        },
        StreamEvent::TextDelta {
            index: 0,
            text: text.into(),
        },
        StreamEvent::ContentBlockStop { index: 0 },
        StreamEvent::MessageDelta {
            stop_reason: Some("end_turn".into()),
            usage: None,
        },
        StreamEvent::MessageStop,
    ]
}
fn record(n: usize, repaired: bool) -> Value {
    let id = format!("TASK-X-{n:03}");
    json!({"subject":id,"summary":"inspected unit","evidence":[{"kind":"inspection","summary":"read source"}],
        "task":{"task_id":id,"file_name":format!("{id}.md"),"implements":["REQ-1"],"source_sections":[if repaired {"2"} else {"1"}]}})
}
fn seeded(repair: bool) -> (tempfile::TempDir, Arc<RecordLanding>) {
    let temp = tempfile::tempdir().unwrap();
    let records = Arc::new(
        RecordLanding::open(
            temp.path().join("records"),
            "call".into(),
            RecordKind::Skeleton,
            vec![],
            false,
        )
        .unwrap(),
    );
    records.seed_skeleton(&json!({"schema_version":1,"acceptance_digest":"","tasks":(1..=19).map(|n| record(n,false)["task"].clone()).collect::<Vec<_>>()}), false).unwrap();
    records.require_new_landings(repair).unwrap();
    (temp, records)
}
fn runner(
    records: Arc<RecordLanding>,
    rounds: Vec<(Vec<Value>, Vec<StreamEvent>)>,
    ceiling: bool,
) -> (SubagentRunner, Arc<ReplyProvider>) {
    let landing = Arc::new(AuditLanding::new(Arc::new(Bridge(records)), None));
    let provider = Arc::new(ReplyProvider {
        landing: landing.clone(),
        rounds: Mutex::new(rounds.into()),
        requests: Mutex::new(vec![]),
    });
    let root = std::env::current_dir().unwrap();
    let registry = Arc::new(archon_core::dispatch::create_default_registry(
        root.clone(),
        None,
    ));
    let context = ToolContext {
        working_dir: root,
        session_id: uuid::Uuid::new_v4().to_string(),
        audit_landing: Some(landing),
        workflow_read_guard: ceiling.then(|| {
            Arc::new(
                archon_tools::workflow_read_guard::WorkflowReadGuard::shell_only(
                    &archon_tools::workflow_read_guard::WorkflowReadGuardSettings {
                        read_only_soft_call_ceiling: 0,
                        read_only_hard_call_ceiling: 1,
                        ..Default::default()
                    },
                ),
            )
        }),
        ..Default::default()
    };
    let runner = SubagentRunner::new(
        provider.clone(),
        "Author task records".into(),
        registry.tool_definitions(),
        registry,
        context,
        "scripted".into(),
        8,
        30,
        Arc::new(AgentConfig::default()),
        Arc::new(IdentityProvider::new(
            IdentityMode::Clean,
            "test".into(),
            String::new(),
            String::new(),
        )),
    );
    (runner, provider)
}

#[tokio::test]
async fn all_reply_classes_compose_real_retained_records_or_retry() {
    for text in [
        r#"{"records_landed":14}"#,
        "Repaired.\n{\"records_landed\":19}",
        "Repaired tasks.",
        "",
    ] {
        let (_temp, records) = seeded(true);
        let new = (1..=14).map(|n| record(n, true)).collect();
        let (runner, provider) = runner(records.clone(), vec![(new, reply(text))], false);
        let completion = runner
            .run_with_completion("Repair the open findings")
            .await
            .unwrap();
        let artifact: Value =
            serde_json::from_str(&compose_raw_reply(&records, &completion.content).unwrap())
                .unwrap();
        assert_eq!(artifact["tasks"].as_array().unwrap().len(), 19);
        for n in 0..19 {
            assert_eq!(
                artifact["tasks"][n]["source_sections"],
                json!([if n < 14 { "2" } else { "1" }])
            );
        }
        assert_eq!(provider.requests.lock().unwrap().len(), 1);
        assert_eq!(completion.stop_reason.as_deref(), Some("end_turn"));
    }
    for text in [
        r#"{"records_landed":19}"#,
        r#"{"records_landed":0}"#,
        "Done.\n{\"records_landed\":19}",
        "Done.",
        "",
    ] {
        let (_temp, records) = seeded(true);
        assert!(
            records.retained_reply().unwrap().is_none(),
            "seeds are not new landings"
        );
        let (runner, provider) = runner(
            records.clone(),
            vec![
                (vec![], reply(text)),
                (vec![record(1, true)], reply(r#"{"records_landed":1}"#)),
            ],
            false,
        );
        let completion = runner.run("Repair the open findings").await.unwrap();
        assert_eq!(provider.requests.lock().unwrap().len(), 2);
        let artifact: Value =
            serde_json::from_str(&compose_raw_reply(&records, &completion).unwrap()).unwrap();
        assert_eq!(artifact["tasks"][0]["source_sections"], json!(["2"]));
    }
}

#[tokio::test]
async fn seed_only_receipts_and_prose_exhaust_the_existing_retry_budget() {
    for text in [
        r#"{"records_landed":19}"#,
        r#"{"records_landed":0}"#,
        "Done.",
        "",
    ] {
        let (_temp, records) = seeded(true);
        let (runner, provider) = runner(
            records,
            (0..3).map(|_| (vec![], reply(text))).collect(),
            false,
        );
        assert!(
            runner
                .run("Repair")
                .await
                .unwrap_err()
                .to_string()
                .contains("incomplete landed artifact")
        );
        assert_eq!(provider.requests.lock().unwrap().len(), 3);
    }
}

#[test]
fn receipt_counts_new_subjects_excluding_seeds_and_repeated_landings() {
    let (_temp, records) = seeded(true);
    assert!(
        records
            .hint()
            .unwrap()
            .contains("0 new landings in this call")
    );
    assert!(records.compose_reply(&json!({"records_landed":0})).is_err());
    records.land(record(1, true)).unwrap();
    records.land(record(1, true)).unwrap();
    assert_eq!(records.new_landing_count().unwrap(), 1);
    assert!(records.hint().unwrap().ends_with(r#"{"records_landed":1}"#));
    assert!(
        records
            .compose_reply(&json!({"records_landed":19}))
            .is_err()
    );
    assert!(
        records
            .compose_reply(&json!({"records_landed":1,"extra":true}))
            .is_err()
    );
    assert_eq!(
        records.compose_reply(&json!({"records_landed":1})).unwrap()["tasks"]
            .as_array()
            .unwrap()
            .len(),
        19
    );
    let (_temp, first) = seeded(false);
    assert_eq!(
        first.compose_reply(&json!({"records_landed":0})).unwrap()["tasks"]
            .as_array()
            .unwrap()
            .len(),
        19
    );
}

#[tokio::test]
async fn forced_answer_at_read_ceiling_uses_the_same_landing_contract() {
    for has_new in [false, true] {
        let (_temp, records) = seeded(true);
        let read = vec![
            StreamEvent::MessageStart {
                id: "read".into(),
                model: "scripted".into(),
                usage: Usage::default(),
            },
            StreamEvent::ContentBlockStart {
                index: 0,
                block_type: ContentBlockType::ToolUse,
                tool_use_id: Some("read".into()),
                tool_name: Some("Read".into()),
            },
            StreamEvent::InputJsonDelta {
                index: 0,
                partial_json: json!({"file_path":"Cargo.toml"}).to_string(),
            },
            StreamEvent::ContentBlockStop { index: 0 },
            StreamEvent::MessageStop,
        ];
        let mut rounds = vec![(
            if has_new {
                vec![record(1, true)]
            } else {
                vec![]
            },
            read,
        )];
        rounds
            .extend((0..if has_new { 1 } else { 3 }).map(|_| (vec![], reply("Finished reading."))));
        let (runner, provider) = runner(records.clone(), rounds, true);
        let completion = runner.run("Inspect and repair").await;
        if has_new {
            assert!(compose_raw_reply(&records, &completion.unwrap()).is_ok());
        } else {
            assert!(
                completion
                    .unwrap_err()
                    .to_string()
                    .contains("incomplete landed artifact")
            );
        }
        let requests = provider.requests.lock().unwrap();
        assert!(
            serde_json::to_string(&requests[1].messages)
                .unwrap()
                .contains("Inspection tools are no longer available")
        );
    }
}

#[test]
fn preparation_transports_open_findings_into_the_real_landing_contract() {
    for repair in [false, true] {
        let temp = tempfile::tempdir().unwrap();
        let store = WorkflowV2ResultStore::new(temp.path().join("v2"));
        let execution = WorkflowV2CallExecution {
            call: WorkflowV2HostCall {
                id: "author".into(),
                method: WorkflowV2HostMethod::Agent,
                write_mode: None,
                options: WorkflowV2HostOptions {
                    extra: [
                        ("recordLanding".into(), json!("skeleton")),
                        ("openFindings".into(), if repair { json!(["repair task"]) } else { json!([]) }),
                        ("previousSkeleton".into(), json!({"schema_version":1,"acceptance_digest":"","tasks":[record(1,false)["task"]]})),
                    ].into(),
                    ..Default::default()
                },
            },
            input: json!({}), depends_on: vec![],
        };
        let mut request =
            archon_workflow::v2::call_data::v2_agent_request("author", None, &execution, None);
        let records = prepare(&mut request, Some(&store)).unwrap().unwrap();
        let bridge = Bridge(records.clone());
        use archon_tools::audit_landing::LandingHost;
        assert_eq!(
            bridge.complete(&json!({"records_landed":0})).is_err(),
            repair
        );
        assert!(bridge.retained_completion().is_none());
        assert_eq!(records.new_landing_count().unwrap(), 0);
        assert!(request.task.contains("seeds do not count"));
        assert!(compose_raw_reply(&records, "Done.").is_err());
        bridge.land(record(1, true)).unwrap();
        bridge.complete(&json!({"records_landed":1})).unwrap();
        let artifact: Value = serde_json::from_str(
            &compose_raw_reply(&records, "Done.\n{\"records_landed\":19}").unwrap(),
        )
        .unwrap();
        assert_eq!(artifact["tasks"][0]["source_sections"], json!(["2"]));
    }
}

#[test]
fn removing_a_seeded_draft_task_counts_as_this_calls_landing() {
    let (_temp, records) = seeded(true);
    records
        .land(json!({"subject":"TASK-X-001","remove":true}))
        .unwrap();
    assert_eq!(records.new_landing_count().unwrap(), 1);
    let artifact = records.compose_reply(&json!({"records_landed":1})).unwrap();
    assert_eq!(artifact["tasks"].as_array().unwrap().len(), 18);
    assert!(
        artifact["tasks"]
            .as_array()
            .unwrap()
            .iter()
            .all(|task| task["task_id"] != "TASK-X-001")
    );
}

#[test]
fn non_skeleton_raw_replies_without_receipts_keep_their_final_text() {
    for kind in [RecordKind::Review, RecordKind::Verify] {
        let temp = tempfile::tempdir().unwrap();
        let records = RecordLanding::open(
            temp.path().join("records"),
            "call".into(),
            kind,
            vec![],
            false,
        )
        .unwrap();
        for text in ["Review complete.", r#"{"status":"accepted","data":{}}"#] {
            assert_eq!(compose_raw_reply(&records, text).unwrap(), text);
        }
    }
}

#[path = "workflow_stage_landing_dispatch_tests.rs"]
mod dispatch_tests;

#[test]
fn landing_contract_refusal_retains_valid_tasks_even_with_pending_rejection() {
    for text in [
        "Done.",
        r#"{"records_landed":99}"#,
        r#"{"records_landed":0}"#,
    ] {
        let (_temp, records) = seeded(true);
        records.land(record(1, true)).unwrap();
        let mut invalid = record(2, false);
        invalid["task"]["source_sections"] = json!(["1", "1"]);
        assert!(records.land(invalid).is_err());
        let error = compose_raw_reply(&records, text).unwrap_err();
        let result = refused_reply("skeleton-author-2", error, Some(&records));
        assert_eq!(result.data["invalid_answer"], true);
        assert_eq!(result.data["retainedLandings"], true);
        let retained: Value =
            serde_json::from_str(result.data["content"].as_str().unwrap()).unwrap();
        assert_eq!(retained["tasks"].as_array().unwrap().len(), 19);
        assert_eq!(retained["tasks"][0]["source_sections"], json!(["2"]));
        assert!(result.summary.contains("failed"));
    }
}

#[test]
fn zero_change_landing_refusal_still_carries_the_retained_store() {
    let (_temp, records) = seeded(true);
    records.land(record(1, false)).unwrap();
    assert_eq!(records.new_landing_count().unwrap(), 0);
    let error = compose_raw_reply(&records, r#"{"records_landed":0}"#).unwrap_err();
    let result = refused_reply("skeleton-author-2", error, Some(&records));
    assert_eq!(result.data["invalid_answer"], true);
    assert_eq!(result.data["retainedLandings"], true);
    let retained: Value = serde_json::from_str(result.data["content"].as_str().unwrap()).unwrap();
    assert_eq!(retained["tasks"].as_array().unwrap().len(), 19);
    assert_eq!(retained["tasks"][0]["source_sections"], json!(["1"]));
}

#[path = "pipeline_landing_e2e_tests.rs"]
mod pipeline_e2e_tests;
