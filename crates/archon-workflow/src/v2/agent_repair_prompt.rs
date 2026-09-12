//! Lossless repair context for content-local validation failures.
use super::{
    FINAL_OUTPUT_RULE, RESULT_SCHEMA, WorkflowV2AgentError, WorkflowV2AgentRequest, truncate_chars,
};
use serde_json::Value;

pub(super) fn build(
    request: &WorkflowV2AgentRequest,
    invalid_output: &str,
    error: &WorkflowV2AgentError,
) -> String {
    // Use the very same envelope tolerance as parse_agent_output. A fenced,
    // prose-wrapped, or repairable JSON reply already reached validation; a
    // stricter parser here would throw away the answer on its repair turn.
    // Keep the raw reply below, using normalization only to classify and locate.
    let parsed =
        super::super::agent_output_normalize::normalize_agent_output(request, invalid_output).ok();
    let local = parsed.is_some()
        && matches!(
            error,
            WorkflowV2AgentError::InvalidResult(_) | WorkflowV2AgentError::MalformedOutput(_)
        );
    let instruction = if local {
        "Return the same object with only the invalid fields corrected. Preserve every other field, \
         record, and piece of evidence. This is a patch of your previous answer, not a new task: \
         do not re-explore the repository or repeat tool calls for this content-local correction. \
         Return the complete corrected JSON object, not a JSON Patch or a diff."
    } else {
        "Return exactly one JSON object matching the required result envelope."
    };
    let context = parsed
        .as_ref()
        .map(|value| fault_context(invalid_output, value, &error.to_string()))
        .unwrap_or_else(|| truncate_chars(invalid_output, 2_000));
    let previous = if parsed.is_some() {
        format!("Fault context:\n{context}\n\nPrevious JSON in full:\n{invalid_output}")
    } else {
        format!("Previous invalid output excerpt:\n{context}")
    };
    let audit_hint = crate::repository_audit::landing::current()
        .map(|landing| landing.hint().unwrap_or_else(|e|format!("Cannot recover landed records: {e}"))).unwrap_or_default();
    let target_files = serde_json::to_string(&request.target_files).unwrap_or_default();
    let target_scopes = serde_json::to_string(&request.target_ownership_scopes).unwrap_or_default();
    let final_output_rule = if request.is_write_capable() {
        FINAL_OUTPUT_RULE
    } else {
        ""
    };
    format!(
        "The previous workflow V2 agent response for call '{}' was invalid.\n\n\
         Error: {error}\n\n\
         {instruction}\n\
         Do not include markdown fences, restored-context summaries, confirmation questions, \
         provider names, model names, or plan-only text.\n\n\
         Declared target_files: {target_files}\n\
         Declared target ownership scopes: {target_scopes}\n\
         Do not edit or claim repository files outside that ownership.\n\n\
         Task:\n{}\n\n\
         Required JSON Result Envelope:\n{RESULT_SCHEMA}\n\n\
         {previous}\n\n{audit_hint}\n{final_output_rule}\n",
        request.call.id, request.task,
    )
}

fn fault_context(output: &str, value: &Value, error: &str) -> String {
    let mut faults = Vec::new();
    find_faults(value, "", error, &mut faults);
    if faults.is_empty() {
        return format!(
            "No quoted value could be located; output head:\n{}",
            truncate_chars(output, 2_000)
        );
    }
    faults
        .into_iter()
        .map(|(path, text)| {
            let window =
                string_window(output, &text).unwrap_or_else(|| truncate_chars(output, 2_000));
            format!("JSON path (RFC 6901): {path}\nOffending value: {text:?}\n{window}")
        })
        .collect::<Vec<_>>()
        .join("\n\n")
}

/// Match decoded leaf strings, not substrings of unrelated evidence or field
/// names. Debug quoting is what lexical validators emit; JSON quoting covers
/// escaped control characters and validators that serialize the bad value.
fn find_faults(value: &Value, path: &str, error: &str, faults: &mut Vec<(String, String)>) {
    match value {
        Value::String(text) if !text.is_empty() => {
            let encoded = serde_json::to_string(text).unwrap_or_default();
            if error.contains(&format!("{text:?}")) || error.contains(&encoded) {
                faults.push((path.to_owned(), text.clone()));
            }
        }
        Value::Object(object) => {
            for (key, child) in object {
                let key = key.replace('~', "~0").replace('/', "~1");
                find_faults(child, &format!("{path}/{key}"), error, faults);
            }
        }
        Value::Array(array) => {
            for (index, child) in array.iter().enumerate() {
                find_faults(child, &format!("{path}/{index}"), error, faults);
            }
        }
        _ => {}
    }
}

/// Scan JSON string tokens so non-canonical escapes (e.g. é, \/) still
/// resolve to the original byte window instead of falling back to the head.
fn string_window(output: &str, target: &str) -> Option<String> {
    let mut start = None;
    let mut escaped = false;
    for (index, ch) in output.char_indices() {
        let Some(begin) = start else {
            if ch == '"' {
                start = Some(index);
            }
            continue;
        };
        if escaped {
            escaped = false;
        } else if ch == '\\' {
            escaped = true;
        } else if ch == '"' {
            let end = index + 1;
            let token = &output[begin..end];
            if serde_json::from_str::<String>(token).ok().as_deref() == Some(target) {
                return Some(window(output, begin, end));
            }
            start = None;
        }
    }
    None
}

fn window(text: &str, begin: usize, end: usize) -> String {
    let left = text[..begin]
        .char_indices()
        .rev()
        .nth(599)
        .map(|(i, _)| i)
        .unwrap_or(0);
    let right = text[end..]
        .char_indices()
        .nth(600)
        .map(|(i, _)| end + i)
        .unwrap_or(text.len());
    format!(
        "{}{}{}",
        if left > 0 { "..." } else { "" },
        &text[left..right],
        if right < text.len() { "..." } else { "" }
    )
}
