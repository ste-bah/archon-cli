use serde_json::{Value, json};
use std::collections::HashMap;

const HEADER: &str =
    "The results of every inspection run earlier in this call follow. Inspection is over.";

/// Flatten the conversation before withdrawing inspection tools. The task and
/// final instruction stay intact even when old evidence must be omitted.
pub(super) fn flatten_history(
    messages: &[Value],
    task: &str,
    note: &str,
    budget_tokens: Option<u64>,
) -> Value {
    let mut evidence = Vec::new();
    let mut open_calls = HashMap::new();
    for message in messages {
        let role = message
            .get("role")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let content = message.get("content");
        if role == "user" && content.and_then(Value::as_str) == Some(task) {
            continue;
        }
        collect_evidence(role, content, &mut evidence, &mut open_calls);
    }

    let mut omitted = 0;
    let mut text = render(&evidence, task, note, omitted);
    if let Some(budget) = budget_tokens.filter(|budget| *budget > 0) {
        while estimated_tokens(&text) > budget && !evidence.is_empty() {
            evidence.remove(0);
            omitted += 1;
            text = render(&evidence, task, note, omitted);
        }
    }
    json!({"role": "user", "content": text})
}

/// Each tool call is one evidence item: its header, then its result, paired by
/// `tool_use.id` and `tool_result.tool_use_id`. The budget trim then removes a
/// whole call, never a header without its result or a result without its header.
fn collect_evidence(
    role: &str,
    content: Option<&Value>,
    evidence: &mut Vec<String>,
    open_calls: &mut HashMap<String, usize>,
) {
    match content {
        Some(Value::String(text)) if role == "assistant" && !text.is_empty() => {
            evidence.push(format!("[your earlier note] {text}"));
        }
        Some(Value::Array(blocks)) => {
            for block in blocks {
                match block.get("type").and_then(Value::as_str) {
                    Some("tool_use") => {
                        let name = block.get("name").and_then(Value::as_str).unwrap_or("tool");
                        let input = block.get("input").unwrap_or(&Value::Null);
                        let compact =
                            serde_json::to_string(input).unwrap_or_else(|_| "null".into());
                        if let Some(id) = block.get("id").and_then(Value::as_str) {
                            open_calls.insert(id.to_string(), evidence.len());
                        }
                        evidence.push(format!("### {name} {compact}"));
                    }
                    Some("tool_result") => {
                        let result = result_text(block.get("content"));
                        let result = if block.get("is_error").and_then(Value::as_bool) == Some(true)
                        {
                            format!("error: {result}")
                        } else {
                            result
                        };
                        let call = block
                            .get("tool_use_id")
                            .and_then(Value::as_str)
                            .and_then(|id| open_calls.remove(id));
                        match call {
                            Some(index) => {
                                evidence[index].push('\n');
                                evidence[index].push_str(&result);
                            }
                            None => evidence.push(result),
                        }
                    }
                    Some("text") if role == "assistant" => {
                        if let Some(text) = block.get("text").and_then(Value::as_str) {
                            evidence.push(format!("[your earlier note] {text}"));
                        }
                    }
                    _ => {} // Thinking and non-text protocol blocks are intentionally omitted.
                }
            }
        }
        _ => {} // Earlier user notes are corrective prompts, not inspection evidence.
    }
}

fn result_text(content: Option<&Value>) -> String {
    match content {
        Some(Value::String(text)) => text.clone(),
        Some(Value::Array(blocks)) => blocks
            .iter()
            .filter_map(|block| block.get("text").and_then(Value::as_str))
            .collect::<Vec<_>>()
            .join("\n"),
        _ => String::new(),
    }
}

fn render(evidence: &[String], task: &str, note: &str, omitted: usize) -> String {
    let mut sections = vec![HEADER.to_string()];
    if omitted > 0 {
        sections.push(format!("[{omitted} earlier inspection results omitted]"));
    }
    sections.extend(evidence.iter().cloned());
    sections.push(format!("### Task\n{task}"));
    sections.push(note.to_string());
    sections.join("\n\n")
}

fn estimated_tokens(text: &str) -> u64 {
    crate::agent::autocompact::estimate_message_tokens(&json!({
        "role": "user",
        "content": text,
    }))
}

#[cfg(test)]
mod tests {
    use super::flatten_history;
    use serde_json::json;

    #[test]
    fn flattens_protocol_blocks_and_preserves_order() {
        let history = vec![
            json!({"role":"user","content":"task"}),
            json!({"role":"assistant","content":[
                {"type":"thinking","thinking":"hidden"},
                {"type":"text","text":"first note"},
                {"type":"tool_use","id":"c1","name":"Read","input":{"path":"a"}}
            ]}),
            json!({"role":"user","content":[
                {"type":"tool_result","tool_use_id":"c1","content":[{"type":"text","text":"line 1"},{"type":"text","text":"line 2"}],"is_error":false},
                {"type":"tool_result","tool_use_id":"unknown","content":"failed","is_error":true}
            ]}),
        ];
        let message = flatten_history(&history, "task", "answer now", None);
        let text = message["content"].as_str().unwrap();
        assert!(text.find("first note").unwrap() < text.find("### Read").unwrap());
        assert!(text.contains("### Read {\"path\":\"a\"}\nline 1\nline 2"));
        assert!(text.contains("line 1\nline 2\n\nerror: failed"));
        assert!(text.find("### Task\ntask").unwrap() < text.find("answer now").unwrap());
        assert!(!text.contains("thinking"));
        assert!(!text.contains("tool_use") && !text.contains("tool_result"));
        assert_eq!(message["role"], "user");
    }

    #[test]
    fn pairs_each_result_with_its_call_by_id_across_messages() {
        let history = vec![
            json!({"role":"assistant","content":[
                {"type":"tool_use","id":"a","name":"Read","input":{"n":1}},
                {"type":"tool_use","id":"b","name":"Grep","input":{"n":2}}
            ]}),
            json!({"role":"user","content":[
                {"type":"tool_result","tool_use_id":"b","content":"result b"},
                {"type":"tool_result","tool_use_id":"a","content":"result a","is_error":true}
            ]}),
        ];
        let message = flatten_history(&history, "task", "answer now", None);
        let text = message["content"].as_str().unwrap();
        let first = "### Read {\"n\":1}\nerror: result a";
        let second = "### Grep {\"n\":2}\nresult b";
        assert!(text.contains(first) && text.contains(second));
        assert!(text.find(first).unwrap() < text.find(second).unwrap());
    }

    #[test]
    fn trims_oldest_evidence_until_the_flattened_message_fits() {
        let history = vec![
            json!({"role":"user","content":"task"}),
            json!({"role":"assistant","content":[
                {"type":"tool_use","id":"1","name":"Read","input":{"n":1}},
                {"type":"tool_use","id":"2","name":"Read","input":{"n":2}},
                {"type":"tool_use","id":"3","name":"Read","input":{"n":3}}
            ]}),
        ];
        let budget = super::estimated_tokens(&super::render(
            &[r#"### Read {"n":3}"#.into()],
            "task",
            "answer now",
            2,
        ));
        let flattened = flatten_history(&history, "task", "answer now", Some(budget));
        let text = flattened["content"].as_str().unwrap();
        assert!(text.contains("earlier inspection results omitted"));
        assert!(text.contains("### Read {\"n\":3}"));
        assert!(text.contains("### Task\ntask") && text.ends_with("answer now"));
        assert!(super::estimated_tokens(text) <= budget);
    }

    #[test]
    fn trim_drops_a_whole_call_header_and_result_together() {
        let old_result = format!("old result {}", "x".repeat(200));
        let history = vec![
            json!({"role":"assistant","content":[
                {"type":"tool_use","id":"old","name":"Read","input":{"n":1}},
                {"type":"tool_use","id":"new","name":"Read","input":{"n":2}}
            ]}),
            json!({"role":"user","content":[
                {"type":"tool_result","tool_use_id":"old","content":old_result},
                {"type":"tool_result","tool_use_id":"new","content":"new result"}
            ]}),
        ];
        let kept = "### Read {\"n\":2}\nnew result";
        let budget =
            super::estimated_tokens(&super::render(&[kept.into()], "task", "answer now", 1));
        let flattened = flatten_history(&history, "task", "answer now", Some(budget));
        let text = flattened["content"].as_str().unwrap();
        assert!(text.contains("[1 earlier inspection results omitted]"));
        assert!(!text.contains("{\"n\":1}") && !text.contains("old result"));
        assert!(text.contains(kept));
        assert!(super::estimated_tokens(text) <= budget);
    }
}
