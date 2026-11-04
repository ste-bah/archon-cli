//! Convert the fixture candidate into the incremental landing-tool exchange.
use serde_json::{Value, json};

pub(super) fn skeleton_message(request: &Value, candidate: &str) -> Value {
    let skeleton: Value = serde_json::from_str(candidate).unwrap();
    let tasks = skeleton["tasks"].as_array().unwrap();
    let received = request["messages"]
        .as_array()
        .and_then(|messages| messages.last())
        .and_then(|message| message["content"].as_array())
        .is_some_and(|blocks| blocks.iter().any(|block| block["type"] == "tool_result"));
    let (content, stop) = if received {
        (
            json!([{"type":"text", "text":json!({"records_landed":tasks.len()}).to_string()}]),
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
