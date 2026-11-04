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
    let (content, stop) = if received {
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
