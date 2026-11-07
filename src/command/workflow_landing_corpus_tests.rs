//! D1/D2 live-shape regressions; provenance and replay policy are in the corpus README.
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

fn fixture(name: &str) -> String {
    std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/landing_reply_corpus")
            .join(name),
    )
    .unwrap()
}
fn blocks(name: &str) -> Vec<Value> {
    serde_json::from_str(&fixture(name)).unwrap()
}
fn text(name: &str) -> Value {
    json!({"type":"text","text":fixture(&format!("{name}.txt"))})
}
fn events(blocks: &[Value], stop: &str) -> Vec<StreamEvent> {
    let mut events = vec![StreamEvent::MessageStart {
        id: "corpus".into(),
        model: "corpus".into(),
        usage: Usage::default(),
    }];
    for (index, block) in blocks.iter().enumerate() {
        let index = index as u32;
        let tool = block["type"] == "tool_use";
        events.push(StreamEvent::ContentBlockStart {
            index,
            block_type: if tool {
                ContentBlockType::ToolUse
            } else {
                ContentBlockType::Text
            },
            tool_use_id: tool.then(|| block["id"].as_str().unwrap().into()),
            tool_name: tool.then(|| block["name"].as_str().unwrap().into()),
        });
        events.push(if tool {
            StreamEvent::InputJsonDelta {
                index,
                partial_json: block["input"].to_string(),
            }
        } else {
            StreamEvent::TextDelta {
                index,
                text: block["text"].as_str().unwrap().into(),
            }
        });
        events.push(StreamEvent::ContentBlockStop { index });
    }
    events.push(StreamEvent::MessageDelta {
        stop_reason: Some(stop.into()),
        usage: None,
    });
    events.push(StreamEvent::MessageStop);
    events
}
struct Provider {
    rounds: Mutex<VecDeque<Vec<StreamEvent>>>,
    requests: Mutex<Vec<LlmRequest>>,
}
#[async_trait::async_trait]
impl LlmProvider for Provider {
    fn name(&self) -> &str {
        "corpus"
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
        // An unexpected extra request remains observable without panicking in a runner task.
        let round = self
            .rounds
            .lock()
            .unwrap()
            .pop_front()
            .unwrap_or_else(|| events(&[json!({"type":"text","text":""})], "end_turn"));
        let (tx, rx) = tokio::sync::mpsc::channel(128);
        tokio::spawn(async move {
            for event in round {
                if tx.send(event).await.is_err() {
                    break;
                }
            }
        });
        Ok(rx)
    }
}
struct Replay {
    _temp: tempfile::TempDir,
    records: Arc<RecordLanding>,
    provider: Arc<Provider>,
    runner: SubagentRunner,
}
fn replay(rounds: Vec<Vec<StreamEvent>>, seeds: usize) -> Replay {
    let temp = tempfile::tempdir().unwrap();
    let tasks: Vec<_> = blocks("landings.json")
        .into_iter()
        .take(seeds)
        .map(|b| b["input"]["task"].clone())
        .collect();
    let store = WorkflowV2ResultStore::new(temp.path().join("v2"));
    let execution = WorkflowV2CallExecution {
        call: WorkflowV2HostCall {
            id: "corpus-author".into(),
            method: WorkflowV2HostMethod::Agent,
            write_mode: None,
            options: WorkflowV2HostOptions {
                extra: [
                    ("recordLanding".into(), json!("skeleton")),
                    ("openFindings".into(), json!(["repair task contracts"])),
                    (
                        "previousSkeleton".into(),
                        json!({"schema_version":1,"acceptance_digest":"","tasks":tasks}),
                    ),
                ]
                .into(),
                ..Default::default()
            },
        },
        input: json!({}),
        depends_on: vec![],
    };
    let mut request =
        archon_workflow::v2::call_data::v2_agent_request("Land records", None, &execution, None);
    let records = prepare(&mut request, Some(&store)).unwrap().unwrap();
    let landing = Arc::new(AuditLanding::new(Arc::new(Bridge(records.clone())), None));
    let provider = Arc::new(Provider {
        rounds: Mutex::new(rounds.into()),
        requests: Mutex::new(vec![]),
    });
    let root = temp.path().to_path_buf();
    let mut registry = archon_core::dispatch::create_default_registry(root.clone(), None);
    // Match subagent_executor/run_runner.rs: install this call's capability.
    registry.replace(Box::new(archon_tools::audit_landing::ScopedLandingTool(
        landing.clone(),
    )));
    let registry = Arc::new(registry);
    let runner = SubagentRunner::new(
        provider.clone(),
        "Land records".into(),
        registry.tool_definitions(),
        registry,
        ToolContext {
            working_dir: root,
            session_id: uuid::Uuid::new_v4().to_string(),
            audit_landing: Some(landing),
            ..Default::default()
        },
        "corpus".into(),
        8,
        10,
        Arc::new(AgentConfig::default()),
        Arc::new(IdentityProvider::new(
            IdentityMode::Clean,
            "test".into(),
            String::new(),
            String::new(),
        )),
    );
    Replay {
        _temp: temp,
        records,
        provider,
        runner,
    }
}
fn assert_tasks(records: &RecordLanding, artifact: &Value, expected: &[Value]) {
    let tasks = artifact["tasks"].as_array().unwrap();
    assert_eq!(tasks.len(), expected.len());
    for task in expected {
        // Typed defaults (for example ordering_only:false) are serialized by the store.
        let expected: archon_workflow::task_skeleton::FrozenTask =
            serde_json::from_value(task.clone()).unwrap();
        let expected = serde_json::to_value(expected).unwrap();
        assert!(tasks.contains(&expected), "lost or changed task: {task}");
    }
    assert_eq!(
        records.retained_artifact().unwrap().unwrap()["tasks"],
        artifact["tasks"]
    );
}
async fn receipt_case(name: &str, new: usize, seeds: usize, final_rounds: usize, completes: bool) {
    let all = blocks("landings.json");
    let mut rounds = vec![];
    if new > 0 {
        rounds.push(events(&all[seeds..seeds + new], "tool_use"));
    }
    rounds.extend((0..final_rounds).map(|_| events(&[text(name)], "end_turn")));
    let run = replay(rounds, seeds);
    let answer = run.runner.run("Process the records").await;
    assert_eq!(run.records.new_landing_count().unwrap(), new);
    assert_eq!(
        run.provider.requests.lock().unwrap().len(),
        final_rounds + usize::from(new > 0)
    );
    if name == "malformed" {
        let requests = run.provider.requests.lock().unwrap();
        let feedback = serde_json::to_string(&requests[usize::from(new > 0) + 1].messages).unwrap();
        assert!(
            feedback.contains("incomplete or malformed JSON"),
            "{feedback}"
        );
    }
    if completes {
        let answer = answer.unwrap();
        assert_eq!(
            serde_json::from_str::<Value>(&answer).unwrap(),
            json!({"records_landed":new})
        );
        let artifact: Value =
            serde_json::from_str(&compose_raw_reply(&run.records, &answer).unwrap()).unwrap();
        let expected: Vec<_> = all[..seeds + new]
            .iter()
            .map(|b| b["input"]["task"].clone())
            .collect();
        assert_tasks(&run.records, &artifact, &expected);
    } else {
        assert!(
            answer
                .unwrap_err()
                .to_string()
                .contains("incomplete landed artifact")
        );
        assert!(compose_raw_reply(&run.records, &fixture(&format!("{name}.txt"))).is_err());
        assert_eq!(
            run.records
                .compose(&json!({"records_landed":seeds}))
                .unwrap()["tasks"]
                .as_array()
                .unwrap()
                .len(),
            seeds
        );
    }
}
macro_rules! case {
    ($test:ident, $shape:literal, $new:expr, $seeds:expr, $rounds:expr, $ok:expr) => {
        #[tokio::test]
        async fn $test() {
            receipt_case($shape, $new, $seeds, $rounds, $ok).await;
        }
    };
}
case!(exact_receipt, "exact", 19, 0, 1, true);
case!(exact_r14_receipt, "exact_r14", 16, 0, 1, true);
case!(under_count_receipt, "under_count", 3, 16, 3, true);
case!(whole_store_receipt, "whole_store", 16, 3, 3, true);
case!(seed_only_receipt, "seed_only", 0, 19, 3, false);
case!(prose_and_receipt, "prose_receipt", 19, 0, 1, true);
case!(
    prose_receipt_checks_new_count,
    "prose_receipt",
    13,
    6,
    3,
    true
);
case!(skeleton_r8_refuses, "skeleton_r8", 0, 0, 3, false);
case!(skeleton_r10_refuses, "skeleton_r10", 0, 0, 3, false);
case!(skeleton_keeps_landed_work, "skeleton_r8", 2, 17, 1, true);
case!(receipt_extra_keys, "extra_keys", 14, 5, 3, true);
case!(receipt_count_as_list, "list_receipt", 2, 17, 3, true);
case!(malformed_json, "malformed", 2, 17, 3, true);
#[tokio::test]
async fn tool_call_as_text() {
    let mut rounds = vec![events(&[text("text_tool")], "end_turn")];
    rounds.extend((0..3).map(|_| events(&[], "end_turn")));
    let run = replay(rounds, 0);
    assert!(
        run.runner
            .run("Process records")
            .await
            .unwrap_err()
            .to_string()
            .contains("incomplete landed artifact")
    );
    assert_eq!(run.provider.requests.lock().unwrap().len(), 4);
    let feedback =
        serde_json::to_string(&run.provider.requests.lock().unwrap()[1].messages).unwrap();
    assert!(
        feedback.contains("written as text") && feedback.contains("not run"),
        "{feedback}"
    );
    assert_eq!(run.records.new_landing_count().unwrap(), 0);
    assert!(compose_raw_reply(&run.records, &fixture("text_tool.txt")).is_err());
}

#[tokio::test]
async fn receipt_in_tool_message_is_not_a_final_answer() {
    let run = replay(
        vec![
            events(&blocks("mixed.json"), "tool_use"),
            events(
                &[json!({"type":"text","text":"{\"records_landed\":1}"})],
                "end_turn",
            ),
        ],
        0,
    );
    let answer = run.runner.run("Process records").await.unwrap();
    assert_eq!(run.provider.requests.lock().unwrap().len(), 2);
    assert_eq!(run.records.new_landing_count().unwrap(), 1);
    let artifact: Value =
        serde_json::from_str(&compose_raw_reply(&run.records, &answer).unwrap()).unwrap();
    assert_tasks(
        &run.records,
        &artifact,
        &[blocks("mixed.json")[1]["input"]["task"].clone()],
    );
}
#[tokio::test]
async fn single_record_reply_lands_then_receipt_completes() {
    let run = replay(
        vec![
            events(&[text("valid_record")], "end_turn"),
            events(
                &[json!({"type":"text","text":"{\"records_landed\":1}"})],
                "end_turn",
            ),
        ],
        0,
    );
    let answer = run.runner.run("Process records").await.unwrap();
    assert_eq!(run.provider.requests.lock().unwrap().len(), 2);
    assert_eq!(run.records.new_landing_count().unwrap(), 1);
    let artifact: Value =
        serde_json::from_str(&compose_raw_reply(&run.records, &answer).unwrap()).unwrap();
    let record: Value = serde_json::from_str(&fixture("valid_record.txt")).unwrap();
    assert_tasks(&run.records, &artifact, &[record["task"].clone()]);
}
// P1/A2: successful record replies must consume the same bounded incomplete budget.
#[tokio::test]
async fn repeated_record_replies_complete_within_budget() {
    let run = replay(
        (0..3)
            .map(|_| events(&[text("valid_record")], "end_turn"))
            .collect(),
        0,
    );
    let answer = run.runner.run("Process records").await.unwrap();
    assert_eq!(run.provider.requests.lock().unwrap().len(), 3);
    assert_eq!(run.records.new_landing_count().unwrap(), 1);
    let artifact: Value =
        serde_json::from_str(&compose_raw_reply(&run.records, &answer).unwrap()).unwrap();
    let record: Value = serde_json::from_str(&fixture("valid_record.txt")).unwrap();
    assert_tasks(&run.records, &artifact, &[record["task"].clone()]);
}
async fn invalid_field(field: &str) {
    let block: Value = serde_json::from_str(&fixture(&format!("string_{field}.json"))).unwrap();
    let valid = blocks("landings.json")[0].clone();
    let run = replay(
        vec![
            events(&[valid.clone(), block], "tool_use"),
            events(
                &[json!({"type":"text","text":"{\"records_landed\":1}"})],
                "end_turn",
            ),
        ],
        0,
    );
    // Pending rejection must refuse completion without losing the valid tool landing.
    let error = run
        .runner
        .run("Process records")
        .await
        .unwrap_err()
        .to_string();
    assert!(error.contains("invalid type"), "{error}");
    assert_eq!(run.records.new_landing_count().unwrap(), 1);
    assert!(compose_raw_reply(&run.records, "{\"records_landed\":1}").is_err());
    let retained = run.records.retained_artifact().unwrap().unwrap();
    assert_tasks(&run.records, &retained, &[valid["input"]["task"].clone()]);
    let requests = run.provider.requests.lock().unwrap();
    let feedback = serde_json::to_string(&requests[1].messages).unwrap();
    assert!(
        feedback.contains(field) && feedback.contains("invalid type"),
        "{feedback}"
    );
    assert_eq!(requests.len(), 4);
    let output = std::process::Command::new("node")
        .arg("src/command/workflow_landing_corpus_replay.cjs")
        .arg(retained.to_string())
        .arg(&error)
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}
#[tokio::test]
async fn string_commands_run_refuses_and_retains_work() {
    invalid_field("commands_run").await;
}
#[tokio::test]
async fn string_evidence_refuses_and_retains_work() {
    invalid_field("evidence").await;
}
// P1/A4: an empty end_turn must reach the landing judge and exhaust its budget.
#[tokio::test]
async fn empty_end_turn_refuses() {
    let run = replay((0..3).map(|_| events(&[], "end_turn")).collect(), 0);
    assert!(
        run.runner
            .run("Process records")
            .await
            .unwrap_err()
            .to_string()
            .contains("incomplete landed artifact")
    );
    assert_eq!(run.provider.requests.lock().unwrap().len(), 3);
    assert!(compose_raw_reply(&run.records, "").is_err());
    assert_eq!(run.records.new_landing_count().unwrap(), 0);
}
// P1/A7: output-limit emptiness is an authored failure, never a transport retry.
#[tokio::test]
async fn max_tokens_empty_is_output_limit() {
    let run = replay(vec![events(&[], "max_tokens")], 0);
    let error = run
        .runner
        .run("Process records")
        .await
        .unwrap_err()
        .to_string();
    assert!(
        error.contains("max_tokens") || error.contains("output limit"),
        "{error}"
    );
    assert!(!error.contains("connection"), "{error}");
    assert_eq!(run.provider.requests.lock().unwrap().len(), 1);
    assert!(compose_raw_reply(&run.records, "").is_err());
    assert_eq!(run.records.new_landing_count().unwrap(), 0);
}

#[tokio::test]
async fn empty_replies_complete_from_landed_work() {
    for stop in ["end_turn", "max_tokens"] {
        let all = blocks("landings.json");
        let run = replay(vec![events(&all[17..], "tool_use"), events(&[], stop)], 17);
        let answer = run.runner.run("Process records").await.unwrap();
        assert_eq!(run.provider.requests.lock().unwrap().len(), 2);
        assert_eq!(run.records.new_landing_count().unwrap(), 2);
        assert_eq!(
            serde_json::from_str::<Value>(&answer).unwrap(),
            json!({"records_landed":2})
        );
        let artifact: Value =
            serde_json::from_str(&compose_raw_reply(&run.records, &answer).unwrap()).unwrap();
        let expected: Vec<_> = all.iter().map(|b| b["input"]["task"].clone()).collect();
        assert_tasks(&run.records, &artifact, &expected);
    }
}

#[tokio::test]
async fn original_single_record_with_invalid_contract_refuses() {
    let run = replay(
        (0..3)
            .map(|_| events(&[text("single_record")], "end_turn"))
            .collect(),
        0,
    );
    let error = run
        .runner
        .run("Process records")
        .await
        .unwrap_err()
        .to_string();
    assert!(error.contains("min_instances:1"), "{error}");
    assert_eq!(run.provider.requests.lock().unwrap().len(), 3);
    assert_eq!(run.records.new_landing_count().unwrap(), 0);
    assert!(compose_raw_reply(&run.records, &fixture("single_record.txt")).is_err());
}

#[path = "workflow_landing_corpus_normalize_tests.rs"]
mod normalize_tests;
