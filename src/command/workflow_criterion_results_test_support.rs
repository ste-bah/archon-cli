//! Test support: a canned agent that answers for every acceptance criterion
//! its prompt lists (Batch O, C6b).
//!
//! Since a write call's `accepted` or `noop` result must carry `criterion_results`, a canned
//! implementation reply without them is (rightly) demoted. A harness whose
//! subject is something else answers like a compliant agent: every criterion
//! the prompt's "Acceptance Criteria Results" section lists, reported met,
//! with evidence. Only accepted and no-op JSON envelopes are touched.

const SECTION: &str = "## Acceptance Criteria Results (required)";

/// `content` with a `criterion_results` entry for every criterion the prompt
/// lists; unchanged when there is no such section or the reply is not an
/// accepted JSON object.
pub(crate) fn satisfy(prompt: &str, content: String) -> String {
    let Some(start) = prompt.find(SECTION) else {
        return content;
    };
    let Ok(mut value) = serde_json::from_str::<serde_json::Value>(&content) else {
        return content;
    };
    if !matches!(
        value.get("status").and_then(serde_json::Value::as_str),
        Some("accepted" | "noop")
    ) {
        return content;
    }
    let entries: Vec<serde_json::Value> = prompt[start + SECTION.len()..]
        .lines()
        .skip(1)
        .take_while(|line| !line.trim().is_empty() && !line.starts_with("## "))
        .filter_map(|line| {
            let (head, text) = line.strip_prefix("- ")?.split_once(": ")?;
            let (task, index) = head.rsplit_once(" #")?;
            Some(serde_json::json!({
                "task_id": task,
                "criterion_index": index.parse::<u64>().ok()?,
                "criterion": text,
                "status": "met",
                "evidence": "canned harness: criterion checked",
            }))
        })
        .collect();
    let Some(object) = value.as_object_mut() else {
        return content;
    };
    if !object.get("data").is_some_and(serde_json::Value::is_object) {
        object.insert("data".to_string(), serde_json::json!({}));
    }
    object["data"]["criterion_results"] = serde_json::Value::Array(entries);
    value.to_string()
}

#[test]
fn a_listed_criterion_is_answered_and_other_replies_are_untouched() {
    let prompt = format!(
        "x\n{SECTION}\nReturn data.criterion_results ...\n- TASK-X-001 #1: a holds\n- TASK-X-001 #2: b: c\n\n## Task\n"
    );
    let out = satisfy(
        &prompt,
        r#"{"status":"accepted","data":{"k":1}}"#.to_string(),
    );
    let value: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert_eq!(value["data"]["k"], 1);
    assert_eq!(value["data"]["criterion_results"][1]["criterion"], "b: c");
    assert_eq!(value["data"]["criterion_results"][1]["criterion_index"], 2);
    let noop = satisfy(&prompt, r#"{"status":"noop","data":{"k":1}}"#.to_string());
    let noop_value: serde_json::Value = serde_json::from_str(&noop).unwrap();
    assert_eq!(
        noop_value["data"]["criterion_results"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    let blocked = r#"{"status":"blocked"}"#.to_string();
    assert_eq!(satisfy(&prompt, blocked.clone()), blocked);
    assert_eq!(satisfy("no section", "{}".to_string()), "{}");
}
