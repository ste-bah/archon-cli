use anyhow::Result;
use archon_workflow::{WorkflowV2CallRecord, WorkflowV2Status};
use serde_json::Value;

use super::super::seed_unmapped as unmapped;

const ACCEPTANCE_GATE: &str = "freeze-acceptance";

pub(super) fn acceptance_candidate(record: &WorkflowV2CallRecord) -> Option<&str> {
    let request = record.call.options.host_command.as_ref()?;
    (request.command_id == ACCEPTANCE_GATE
        && record
            .result
            .data
            .get("gateEnvelope")
            .is_some_and(Value::is_object)
        && matches!(
            record.status,
            WorkflowV2Status::Accepted | WorkflowV2Status::NeedsReview | WorkflowV2Status::Noop
        )
        && record.invalidated_by.is_none()
        && record.result.data["gateEnvelope"]
            .get("operational_error")
            .is_none_or(Value::is_null))
    .then(|| request.stdin.as_deref().unwrap_or_default())
}

pub(super) fn authored_criterion(record: &WorkflowV2CallRecord, id: &str) -> Option<String> {
    let task = record.call.options.task.as_deref()?;
    let marker = format!("Author ONLY entry {id}: ");
    let start = task.rfind(&marker)? + marker.len();
    let rest = &task[start..];
    let end = [
        rest.find("\nThe host refused this entry's last answered reply."),
        rest.find("\nAll criterion IDs and text (for consistency):"),
        id.starts_with("SUP-")
            .then(|| rest.find("\nIts covers is exactly [\""))
            .flatten(),
    ]
    .into_iter()
    .flatten()
    .min()
    .unwrap_or(rest.len());
    let authored = rest[..end].trim_end();
    let criterion = if id.starts_with("SUP-") {
        let prefix = authored.find("): ")? + 3;
        &authored[prefix..]
    } else {
        authored
    };
    Some(criterion.to_string())
}

struct ReplyBlock {
    info: String,
    content: String,
    name: Option<String>,
    start: usize,
    end: usize,
}

fn reply_blocks(content: &str) -> Vec<ReplyBlock> {
    let mut lines = Vec::new();
    let mut start = 0;
    for (index, byte) in content.bytes().enumerate() {
        if byte == b'\n' {
            let end = index + 1;
            let mut body_end = index;
            if body_end > start && content.as_bytes()[body_end - 1] == b'\r' {
                body_end -= 1;
            }
            lines.push((start, end, &content[start..body_end]));
            start = end;
        }
    }
    if start < content.len() {
        lines.push((start, content.len(), &content[start..]));
    }
    let mut blocks = Vec::new();
    let mut i = 0;
    while i < lines.len() {
        let indent = lines[i].2.chars().take_while(|c| *c == ' ').count();
        let rest = &lines[i].2[indent..];
        if indent > 3 || !rest.starts_with("```") {
            i += 1;
            continue;
        }
        let fence_len = rest.bytes().take_while(|b| *b == b'`').count();
        if fence_len < 3 {
            i += 1;
            continue;
        }
        let info = rest[fence_len..].trim().to_string();
        if info.contains('`') {
            i += 1;
            continue;
        }
        let mut close = None;
        for j in i + 1..lines.len() {
            let spaces = lines[j].2.chars().take_while(|c| *c == ' ').count();
            let suffix = &lines[j].2[spaces..];
            let ticks = suffix.bytes().take_while(|b| *b == b'`').count();
            if spaces <= 3
                && ticks >= fence_len
                && suffix[ticks..].trim_matches([' ', '\t']).is_empty()
            {
                close = Some(j);
                break;
            }
        }
        let Some(close) = close else {
            i += 1;
            continue;
        };
        let mut block_content = content[lines[i].1..lines[close].0].to_string();
        if block_content.ends_with("\r\n") {
            block_content.truncate(block_content.len() - 2);
        } else if block_content.ends_with(['\n', '\r']) {
            block_content.pop();
        }
        let name = info
            .strip_prefix("check ")
            .filter(|name| !name.is_empty() && !name.chars().any(char::is_whitespace))
            .map(str::to_string);
        blocks.push(ReplyBlock {
            info,
            content: block_content,
            name,
            start: lines[i].0,
            end: lines[close].1,
        });
        i = close + 1;
    }
    blocks
}

/// The JSON object a reply carries: an explicit json fence, or the first
/// top-level object outside any check block.
pub(super) fn extract_object(content: &str) -> String {
    let blocks = reply_blocks(content);
    let body = if let Some(block) = blocks.iter().find(|block| block.info == "json") {
        block.content.clone()
    } else {
        let mut bytes = content.as_bytes().to_vec();
        for block in blocks.iter().filter(|block| block.name.is_some()) {
            bytes[block.start..block.end].fill(b' ');
        }
        let source = String::from_utf8(bytes).expect("mask preserves UTF-8");
        let mut depth = 0usize;
        let mut first = None;
        let mut quoted = false;
        let mut escaped = false;
        for (index, byte) in source.bytes().enumerate() {
            if quoted {
                if escaped {
                    escaped = false;
                } else if byte == b'\\' {
                    escaped = true;
                } else if byte == b'"' {
                    quoted = false;
                }
            } else if byte == b'"' {
                quoted = true;
            } else if byte == b'{' {
                if depth == 0 {
                    first = Some(index);
                }
                depth += 1;
            } else if byte == b'}' && depth > 0 {
                depth -= 1;
                if depth == 0 {
                    return source[first.unwrap()..=index].to_string();
                }
            }
        }
        return source[first.unwrap_or(0)..].trim().to_string();
    };
    match (body.find('{'), body.rfind('}')) {
        (Some(first), Some(last)) if last > first => body[first..=last].to_string(),
        _ => body.trim().to_string(),
    }
}

/// Resolve one referenced block before seed validation and stamping.
pub(super) fn resolve_command_block(entry: &mut Value, content: &str) -> Option<String> {
    let id = entry
        .get("id")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let blocks = reply_blocks(content);
    let named: Vec<_> = blocks.iter().filter(|block| block.name.is_some()).collect();
    let Some(check) = entry.get_mut("check").and_then(Value::as_object_mut) else {
        return named.first().map(|block| {
            format!(
                "acceptance reply contains unreferenced check block {}",
                block.name.as_deref().unwrap()
            )
        });
    };
    if !check.contains_key("command_block") {
        return named.first().map(|block| {
            format!(
                "acceptance reply contains unreferenced check block {}",
                block.name.as_deref().unwrap()
            )
        });
    }
    let name = check
        .get("command_block")
        .and_then(Value::as_str)
        .map(str::to_string);
    if check.contains_key("command") {
        return Some(format!(
            "acceptance entry {id} returned both check.command and check.command_block"
        ));
    }
    let Some(name) = name.filter(|name| !name.is_empty()) else {
        return Some(format!(
            "acceptance entry {id} check.command_block must name a check block"
        ));
    };
    let matches: Vec<_> = named
        .iter()
        .filter(|block| block.name.as_deref() == Some(&name))
        .collect();
    if matches.is_empty() {
        return Some(format!(
            "acceptance entry {id} command_block {name} has no matching check block"
        ));
    }
    if matches.len() > 1 {
        return Some(format!(
            "acceptance reply contains duplicate check block name {name}"
        ));
    }
    if let Some(block) = named
        .iter()
        .find(|block| block.name.as_deref() != Some(&name))
    {
        return Some(format!(
            "acceptance reply contains unreferenced check block {}",
            block.name.as_deref().unwrap()
        ));
    }
    let block = matches[0];
    if block.content.trim().is_empty() {
        return Some(format!("acceptance entry {id} check block {name} is empty"));
    }
    check.insert("command".into(), Value::String(block.content.clone()));
    check.remove("command_block");
    None
}

/// The entry `id` a reply holds, as the script's `unwrapEntry` reads it.
pub(super) fn reply_entry(text: &str, id: &str) -> Option<Value> {
    let parsed: Value = serde_json::from_str(text).ok()?;
    if parsed["id"] == id {
        return Some(parsed);
    }
    match parsed["acceptance"].as_array().map(Vec::as_slice) {
        Some([only]) if only["id"] == id => Some(only.clone()),
        _ => None,
    }
}

pub(super) fn reply_entry_with_blocks(content: &str, id: &str) -> Option<Value> {
    let text = extract_object(content);
    let mut entry = reply_entry(&text, id)?;
    resolve_command_block(&mut entry, content)
        .is_none()
        .then_some(entry)
}

/// This build's freeze shape refusals of one carried entry.
pub(super) fn entry_refusals(id: &str, entry: &Value) -> Result<Vec<String>> {
    let candidate = serde_json::to_vec(&serde_json::json!({ "entries": [entry] }))?;
    Ok(crate::command::workflow::element_shape_defects(
        &candidate,
        &crate::command::workflow::ENTRY_SHAPE,
    )
    .iter()
    .map(|defect| {
        format!(
            "acceptance entry '{id}' was refused: {}",
            defect.message.replacen("entries/0", "entry", 1)
        )
    })
    .collect())
}

/// The entries of a recorded acceptance candidate, in its own order.
pub(super) fn candidate_entries(stdin: &str, gate: &str) -> Result<Vec<(String, Value, bool)>> {
    let field = format!("v2/results/{gate}.call.options.host_command.stdin");
    let document: Value =
        serde_json::from_str(stdin).map_err(|e| unmapped(&field, &e.to_string()))?;
    let mut out = Vec::new();
    for (list, supplementary) in [("entries", false), ("supplementary", true)] {
        let Some(items) = document.get(list) else {
            if list == "entries" {
                return Err(unmapped(&field, "the candidate has no entries list"));
            }
            continue;
        };
        for item in items.as_array().into_iter().flatten() {
            let id = item["id"]
                .as_str()
                .ok_or_else(|| unmapped(&field, "a candidate entry has no string id"))?;
            out.push((id.to_string(), item.clone(), supplementary));
        }
    }
    Ok(out)
}

#[cfg(test)]
#[path = "workflow_decompose_seed_record_tests.rs"]
mod tests;
