//! Convert the fixture candidate into the incremental landing-tool exchange.
use serde_json::{Value, json};

pub(super) fn skeleton_message(request: &Value, candidate: &str) -> Value {
    let skeleton: Value = serde_json::from_str(candidate).unwrap();
    let mut tasks = skeleton["tasks"].as_array().unwrap().clone();
    let rejected_duplicate = tasks.iter().any(|task| {
        task["source_sections"]
            .as_array()
            .is_some_and(|sections| sections.iter().filter(|s| **s == json!("2")).count() > 1)
    });
    let refused_final = request["messages"]
        .as_array()
        .into_iter()
        .flatten()
        .any(|m| {
            let text = m["content"].to_string();
            text.contains("Landed artifact incomplete:")
                && text.contains("TASK-X-010")
                && text.contains("lists '2' twice")
        });
    if rejected_duplicate && refused_final {
        tasks.retain(|task| task["task_id"] == "TASK-X-010");
        let sections = tasks[0]["source_sections"].as_array_mut().unwrap();
        let mut seen = std::collections::BTreeSet::new();
        sections.retain(|s| seen.insert(s.to_string()));
    }
    // The test double follows the patch contract: send only task changes.
    let previous = request["messages"]
        .as_array()
        .into_iter()
        .flatten()
        .flat_map(|m| {
            if let Some(text) = m["content"].as_str() {
                vec![text]
            } else {
                m["content"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|b| b["text"].as_str())
                    .collect()
            }
        })
        .find_map(|text| {
            let path = text
                .split_once("The previous candidate is in ")?
                .1
                .split_once(" (sha256 ")?
                .0;
            serde_json::from_slice::<Value>(&std::fs::read(path).ok()?).ok()
        });
    if let Some(previous) = previous {
        let normalize = |task: &Value| {
            serde_json::from_value::<archon_workflow::task_skeleton::FrozenTask>(task.clone())
                .ok()
                .and_then(|task| serde_json::to_value(task).ok())
        };
        tasks.retain(|task| {
            !previous["tasks"]
                .as_array()
                .into_iter()
                .flatten()
                .any(|old| {
                    old["task_id"] == task["task_id"]
                        && normalize(old)
                            .zip(normalize(task))
                            .is_some_and(|(old, new)| old == new)
                })
        });
    }
    let received = request["messages"]
        .as_array()
        .and_then(|messages| messages.last())
        .and_then(|message| message["content"].as_array())
        .is_some_and(|blocks| blocks.iter().any(|block| block["type"] == "tool_result"));
    let receipt_count = request["messages"]
        .as_array()
        .into_iter()
        .flatten()
        .rev()
        .filter_map(|message| message["content"].as_array())
        .flat_map(|blocks| blocks.iter().rev())
        .filter(|block| block["type"] == "tool_result")
        .filter_map(|block| block["content"].as_str())
        .find_map(|text| {
            let (_, completion) = text.rsplit_once("your final reply is exactly ")?;
            serde_json::from_str::<Value>(completion).ok()?["records_landed"].as_u64()
        })
        .unwrap_or(tasks.len() as u64);
    let (content, stop) = if received || tasks.is_empty() {
        (
            json!([{"type":"text", "text":json!({"records_landed":receipt_count}).to_string()}]),
            "end_turn",
        )
    } else {
        let name = request["tools"]
            .as_array()
            .unwrap()
            .iter()
            .find(|tool| !tool["input_schema"]["properties"]["task"].is_null())
            .and_then(|tool| tool["name"].as_str())
            .expect("skeleton author has a task landing tool");
        let calls = tasks
            .iter()
            .enumerate()
            .map(|(index, task)| {
                json!({
                    "type":"tool_use", "id":format!("skeleton-{index}"), "name":name,
                    "input":{"subject":task["task_id"], "task":task, "replace":true,
                        "evidence":[{"kind":"inspection", "summary":"Read the fixture PRD."}],
                        "summary":"Land the fixture task."}
                })
            })
            .collect::<Vec<_>>();
        (json!(calls), "tool_use")
    };
    json!({"id":"msg-local", "type":"message", "role":"assistant", "content":content,
        "model":"offline", "stop_reason":stop, "stop_sequence":null,
        "usage":{"input_tokens":1, "output_tokens":1}})
}

pub(super) fn stream_message(message: &Value) -> String {
    let mut start = message.clone();
    start["content"] = json!([]);
    start["stop_reason"] = Value::Null;
    let mut events = vec![json!({"type":"message_start", "message":start})];
    for (index, block) in message["content"].as_array().unwrap().iter().enumerate() {
        let mut initial = block.clone();
        let delta = if block["type"] == "tool_use" {
            initial["input"] = json!({});
            json!({"type":"input_json_delta", "partial_json":block["input"].to_string()})
        } else {
            initial["text"] = json!("");
            json!({"type":"text_delta", "text":block["text"]})
        };
        events.push(json!({"type":"content_block_start", "index":index, "content_block":initial}));
        events.push(json!({"type":"content_block_delta", "index":index, "delta":delta}));
        events.push(json!({"type":"content_block_stop", "index":index}));
    }
    events.push(
        json!({"type":"message_delta", "delta":{"stop_reason":message["stop_reason"],
        "stop_sequence":null}, "usage":{"output_tokens":1}}),
    );
    events.push(json!({"type":"message_stop"}));
    events
        .iter()
        .map(|event| {
            format!(
                "event: {}\ndata: {event}\n\n",
                event["type"].as_str().unwrap()
            )
        })
        .collect()
}

#[cfg(test)]
mod receipt_tests {
    use super::*;

    #[test]
    fn patch_completion_uses_the_seeded_store_count_from_the_receipt() {
        let request = json!({"messages":[{"role":"user", "content":[
            {"type":"tool_result", "content":"Host retained 2 records. When no task is left to land, your final reply is exactly {\"records_landed\":2}"}
        ]}]});
        let reply = skeleton_message(&request, r#"{"tasks":[{"task_id":"TASK-X-020"}]}"#);
        assert_eq!(reply["content"][0]["text"], r#"{"records_landed":2}"#);
        assert_eq!(reply["stop_reason"], "end_turn");
    }
}

#[cfg(test)]
mod rejected_reply_tests {
    use super::*;

    #[test]
    fn r17_v1_final_is_refused_then_only_the_duplicate_is_removed() {
        let candidate = json!({"tasks":[
            {"task_id":"TASK-X-010", "source_sections":["0","2","2"], "implements":["REQ-A"]},
            {"task_id":"TASK-X-020", "source_sections":["3"], "implements":["REQ-B"]}
        ]});
        let tools =
            json!([{"name":"land-skeleton-record", "input_schema":{"properties":{"task":{}}}}]);
        let mut request = json!({"tools":tools, "messages":[{"role":"user", "content":"author"}]});
        let first = skeleton_message(&request, &candidate.to_string());
        assert_eq!(first["content"].as_array().unwrap().len(), 2);
        request["messages"].as_array_mut().unwrap().push(json!({"role":"user", "content":[
            {"type":"tool_result", "content":"When no task is left to land, your final reply is exactly {\"records_landed\":1}"}
        ]}));
        assert_eq!(
            skeleton_message(&request, &candidate.to_string())["content"][0]["text"],
            r#"{"records_landed":1}"#
        );
        request["messages"].as_array_mut().unwrap().push(json!({"role":"user", "content":"Landed artifact incomplete: TASK-X-010 lists '2' twice"}));
        let repair = skeleton_message(&request, &candidate.to_string());
        assert_eq!(repair["stop_reason"], "tool_use");
        assert_eq!(repair["content"].as_array().unwrap().len(), 1);
        let mut expected = candidate["tasks"][0].clone();
        expected["source_sections"] = json!(["0", "2"]);
        assert_eq!(repair["content"][0]["input"]["task"], expected);
    }
}

#[cfg(test)]
mod retained_store_tests {
    use super::*;
    use archon_workflow::v2::record_landing::{RecordKind, RecordLanding};

    #[test]
    fn offline_skeleton_relands_seeded_tasks_and_uses_the_actual_new_count() {
        let temp = tempfile::tempdir().unwrap();
        let records = RecordLanding::open(
            temp.path().join("records"),
            "call".into(),
            RecordKind::Skeleton,
            vec![],
            false,
        )
        .unwrap();
        let candidate = json!({"schema_version":1,"acceptance_digest":"","tasks":[
            {"task_id":"TASK-X-001","file_name":"TASK-X-001.md","implements":["REQ-1"]},
            {"task_id":"TASK-X-002","file_name":"TASK-X-002.md","implements":["REQ-2"]}
        ]});
        let candidate = serde_json::to_value(
            serde_json::from_value::<archon_workflow::task_skeleton::TaskSkeleton>(candidate)
                .unwrap(),
        )
        .unwrap();
        let mut previous = candidate.clone();
        for task in previous["tasks"].as_array_mut().unwrap() {
            task["source_sections"] = json!(["1"]);
        }
        records.seed_skeleton(&previous, false).unwrap();
        records.require_new_landings(true).unwrap();
        let mut request = json!({"messages":[{"role":"user","content":"Author records"}],
            "tools":[{"name":"land-skeleton-record","input_schema":{"properties":{"task":{}}}}]});
        let landed = skeleton_message(&request, &candidate.to_string());
        let mut results = Vec::new();
        for call in landed["content"].as_array().unwrap() {
            records.land(call["input"].clone()).unwrap();
            results.push(json!({"type":"tool_result","content":records.hint().unwrap()}));
            // A duplicate tool call does not add another new landing.
            records.land(call["input"].clone()).unwrap();
        }
        assert_eq!(records.new_landing_count().unwrap(), 2);
        request["messages"]
            .as_array_mut()
            .unwrap()
            .push(json!({"role":"user","content":results}));
        let reply = skeleton_message(&request, &candidate.to_string());
        let receipt: Value =
            serde_json::from_str(reply["content"][0]["text"].as_str().unwrap()).unwrap();
        assert_eq!(receipt, json!({"records_landed":2}));
        assert_eq!(
            records.compose_reply(&receipt).unwrap()["tasks"],
            candidate["tasks"]
        );
    }
}

#[cfg(test)]
mod changed_task_tests {
    use super::*;
    #[test]
    fn offline_skeleton_patch_skips_unchanged_tasks_from_previous_context() {
        let temp = tempfile::tempdir().unwrap();
        let previous = json!({"tasks":[
            {"task_id":"TASK-X-001","file_name":"TASK-X-001.md","implements":["REQ-1"]},
            {"task_id":"TASK-X-002","file_name":"TASK-X-002.md","implements":["REQ-2"]}
        ]});
        let path = temp.path().join("previous.json");
        std::fs::write(&path, previous.to_string()).unwrap();
        let mut candidate = previous.clone();
        candidate["tasks"][1]["source_sections"] = json!(["2"]);
        let request = json!({"messages":[{"content":format!("The previous candidate is in {} (sha256 fixture; host-written context for this call: read it by its exact path).",path.display())}],
            "tools":[{"name":"land-skeleton-record","input_schema":{"properties":{"task":{}}}}]});
        let message = skeleton_message(&request, &candidate.to_string());
        let calls = message["content"].as_array().unwrap();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0]["input"]["subject"], "TASK-X-002");
        assert_ne!(calls[0]["input"]["task"], previous["tasks"][1]);
        assert_eq!(calls[0]["input"]["task"], candidate["tasks"][1]);
        let unchanged = skeleton_message(&request, &previous.to_string());
        assert_eq!(unchanged["stop_reason"], "end_turn");
        assert_eq!(unchanged["content"][0]["text"], r#"{"records_landed":0}"#);
    }
}
