use serde_json::Value;

pub(crate) struct ReplyBlock {
    pub(crate) info: String,
    pub(crate) content: String,
    pub(crate) start: usize,
    pub(crate) end: usize,
    pub(crate) opened_line: usize,
    pub(crate) closed: bool,
}

fn complete_json_object_end(text: &str) -> Option<usize> {
    let start = text.find(|ch: char| !ch.is_whitespace())?;
    if !text[start..].starts_with('{') {
        return None;
    }
    let bytes = text.as_bytes();
    let mut stack = Vec::new();
    let (mut quoted, mut escaped) = (false, false);
    for index in start..bytes.len() {
        let byte = bytes[index];
        if quoted {
            if escaped {
                escaped = false;
            } else if byte == b'\\' {
                escaped = true;
            } else if byte == b'"' {
                quoted = false;
            }
            continue;
        }
        match byte {
            b'"' => quoted = true,
            b'{' | b'[' => stack.push(byte),
            b'}' | b']' => {
                let expected = if byte == b'}' { b'{' } else { b'[' };
                if stack.pop() != Some(expected) {
                    return None;
                }
                if stack.is_empty() {
                    let end = index + 1;
                    serde_json::from_str::<Value>(&text[start..end]).ok()?;
                    return Some(end);
                }
            }
            _ => {}
        }
    }
    None
}

pub(crate) fn strip_reply_markers(content: &str, markers: &[String]) -> String {
    strip_reply_markers_with_map(content, markers).0
}

pub(crate) fn strip_reply_markers_with_map(
    content: &str,
    markers: &[String],
) -> (String, Vec<usize>) {
    if markers.iter().all(String::is_empty) {
        return (content.to_string(), Vec::new());
    }
    let mut output = String::with_capacity(content.len());
    let mut inserted_newline_lines = Vec::new();
    let mut in_fence = false;
    let mut fence_len = 0usize;
    for line in content.split_inclusive(['\n', '\r']) {
        if in_fence {
            output.push_str(line);
            let body = line.trim_end_matches(['\r', '\n']);
            let spaces = body.chars().take_while(|ch| *ch == ' ').count();
            let suffix = &body[spaces..];
            let ticks = suffix.bytes().take_while(|byte| *byte == b'`').count();
            if spaces <= 3
                && ticks >= fence_len
                && suffix[ticks..].trim_matches([' ', '\t']).is_empty()
            {
                in_fence = false;
            }
            continue;
        }
        let ending_len = line
            .bytes()
            .rev()
            .take_while(|byte| matches!(byte, b'\r' | b'\n'))
            .count();
        let (_, ending) = line.split_at(line.len() - ending_len);
        let mut owned = line[..line.len() - ending_len].to_string();
        // A marker directly between a completed JSON object and a fence opener
        // may split that line. Other mid-line openers remain invalid.
        let mut fence_at = owned.find("```");
        while let Some(at) = fence_at {
            let prefix = &owned[..at];
            let boundary = complete_json_object_end(prefix);
            let mut rest = boundary.map(|end| &prefix[end..]).unwrap_or("");
            let mut removed = false;
            while !rest.is_empty() {
                if let Some(marker) = markers
                    .iter()
                    .filter(|marker| !marker.is_empty())
                    .find(|marker| rest.starts_with(marker.as_str()))
                {
                    rest = &rest[marker.len()..];
                    removed = true;
                } else {
                    break;
                }
            }
            let allowed = boundary.is_some() && removed && rest.trim().is_empty();
            if allowed {
                let inserted_line =
                    output.matches('\n').count() + owned[..at].matches('\n').count() + 2;
                inserted_newline_lines.push(inserted_line);
                owned.insert(at, '\n');
                break;
            }
            fence_at = owned[at + 3..].find("```").map(|next| at + 3 + next);
        }
        for marker in markers.iter().filter(|marker| !marker.is_empty()) {
            owned = owned.replace(marker, "");
        }
        let body = owned.as_str();
        let opener_line = body.rsplit(['\r', '\n']).next().unwrap_or(body);
        let spaces = opener_line.chars().take_while(|ch| *ch == ' ').count();
        let suffix = &opener_line[spaces..];
        let ticks = suffix.bytes().take_while(|byte| *byte == b'`').count();
        if spaces <= 3 && ticks >= 3 && !suffix[ticks..].contains('`') {
            in_fence = true;
            fence_len = ticks;
        }
        output.push_str(body);
        output.push_str(ending);
    }
    (output, inserted_newline_lines)
}

pub(crate) fn reply_blocks(content: &str) -> Vec<ReplyBlock> {
    reply_blocks_with_markers(content, &[])
}

pub(crate) fn reply_blocks_with_markers(content: &str, markers: &[String]) -> Vec<ReplyBlock> {
    let normalized = strip_reply_markers(content, markers);
    let content = normalized.as_str();
    let mut lines = Vec::new();
    let mut start = 0;
    let bytes = content.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        if matches!(bytes[index], b'\r' | b'\n') {
            let body_end = index;
            let end = if bytes[index] == b'\r' && bytes.get(index + 1) == Some(&b'\n') {
                index + 2
            } else {
                index + 1
            };
            lines.push((start, end, &content[start..body_end]));
            start = end;
            index = end;
        } else {
            index += 1;
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
            blocks.push(ReplyBlock {
                info,
                content: String::new(),
                start: lines[i].0,
                end: content.len(),
                opened_line: i + 1,
                closed: false,
            });
            break;
        };
        let mut block_content = content[lines[i].1..lines[close].0].to_string();
        if block_content.ends_with("\r\n") {
            block_content.truncate(block_content.len() - 2);
        } else if block_content.ends_with(['\n', '\r']) {
            block_content.pop();
        }
        blocks.push(ReplyBlock {
            info,
            content: block_content,
            start: lines[i].0,
            end: lines[close].1,
            opened_line: i + 1,
            closed: true,
        });
        i = close + 1;
    }
    blocks
}

pub(crate) use super::acceptance_reply_diagnostics::{
    unclosed_block_refusal, unclosed_block_refusal_with_markers,
};

fn found_fence_labels(blocks: &[ReplyBlock]) -> String {
    const LABELS: usize = 4;
    const LABEL_CHARS: usize = 40;
    let mut labels = Vec::new();
    for block in blocks {
        let label: String = block.info.chars().take(LABEL_CHARS).collect();
        labels.push(if block.info.chars().count() > LABEL_CHARS {
            format!("{label}…")
        } else if label.is_empty() {
            "<empty>".to_string()
        } else {
            label
        });
    }
    let shown = labels
        .iter()
        .take(LABELS)
        .map(|label| format!("'{label}'"))
        .collect::<Vec<_>>();
    let omitted = labels.len().saturating_sub(LABELS);
    if omitted > 0 {
        format!("{} (and {omitted} more)", shown.join(", "))
    } else if shown.is_empty() {
        "none".to_string()
    } else {
        shown.join(", ")
    }
}

/// Resolve one referenced block before seed validation and stamping.
#[cfg(test)]
pub(crate) fn resolve_command_block(entry: &mut Value, content: &str) -> Option<String> {
    resolve_command_block_with_markers(entry, content, &[])
}

pub(crate) fn resolve_command_block_with_markers(
    entry: &mut Value,
    content: &str,
    markers: &[String],
) -> Option<String> {
    let id = entry
        .get("id")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let normalized = strip_reply_markers(content, markers);
    let content = normalized.as_str();
    if let Some(refusal) = unclosed_block_refusal(content) {
        return Some(refusal);
    }
    let blocks = reply_blocks(content);
    // Only the exact info string `check` denotes the command block; `check <something>` is a different fence label.
    let checks: Vec<_> = blocks
        .iter()
        .filter(|block| block.info == "check")
        .collect();
    if checks.len() > 1 {
        return Some("acceptance reply contains more than one check block".into());
    }
    let Some(check) = entry.get_mut("check").and_then(Value::as_object_mut) else {
        return (!checks.is_empty()).then(|| "check block present but check.command_block is not true — put the script only in the block and set command_block true, or remove the block".into());
    };
    let command_block = check.get("command_block");
    let inferred_command_block = !checks.is_empty()
        && check.get("kind").and_then(Value::as_str) == Some("command")
        && !check.contains_key("command")
        && command_block.is_none();
    if (command_block == Some(&Value::Bool(true)) || !checks.is_empty())
        && check.get("kind").and_then(Value::as_str) != Some("command")
    {
        return Some(format!(
            "acceptance entry {id}: check.command_block is only for kind command; a floor check puts its verifier in contract.typed_verifier_command"
        ));
    }
    if command_block == Some(&Value::Bool(true)) && check.contains_key("command") {
        return Some(format!(
            "acceptance entry {id} returned both check.command and check.command_block — remove check.command from the JSON when you use the check block"
        ));
    }
    if !checks.is_empty()
        && command_block != Some(&Value::Bool(true))
        && check.contains_key("command")
    {
        return Some(format!(
            "acceptance entry {id} mixes inline check.command and a check fence; for a multi-line script, use block form with check.command_block true, no inline check.command, and the script in the check fence"
        ));
    }
    if command_block == Some(&Value::Bool(true)) && checks.is_empty() {
        return Some(format!(
            "acceptance entry {id} check.command_block is true but no check block is present — the fence info string must be exactly 'check'; found fence labels: {}",
            found_fence_labels(&blocks)
        ));
    }
    if !checks.is_empty() && command_block != Some(&Value::Bool(true)) && !inferred_command_block {
        return Some("check block present but check.command_block is not true — put the script only in the block and set command_block true, or remove the block".into());
    }
    if command_block.is_some()
        && command_block != Some(&Value::Bool(true))
        && command_block != Some(&Value::Bool(false))
    {
        return Some(format!(
            "acceptance entry {id} check.command_block must be boolean true"
        ));
    }
    let Some(block) = checks.first() else {
        return None;
    };
    if block.content.trim().is_empty() {
        return Some(format!("acceptance entry {id} check block is empty"));
    }
    let command = block.content.replace("\r\n", "\n").replace('\r', "\n");
    check.insert("command".into(), Value::String(command));
    check.remove("command_block");
    None
}
/// The JSON object a reply carries: an explicit json fence, or the first
/// top-level object outside any check block.
#[cfg(test)]
pub(crate) fn extract_object(content: &str) -> String {
    extract_object_with_markers(content, &[])
}

pub(crate) fn extract_object_with_markers(content: &str, markers: &[String]) -> String {
    match extract_object_outcome_with_markers(content, markers) {
        ObjectExtraction::Entry(entry) | ObjectExtraction::ArrayWrappedEntry(entry) => entry,
        ObjectExtraction::NoneFound | ObjectExtraction::SeveralDifferentEntries { .. } => {
            String::new()
        }
    }
}

pub(crate) enum ObjectExtraction {
    Entry(String),
    NoneFound,
    SeveralDifferentEntries { count: usize, array_wrapped: bool },
    ArrayWrappedEntry(String),
}

pub(crate) fn extract_object_outcome_with_markers(
    content: &str,
    markers: &[String],
) -> ObjectExtraction {
    let normalized = strip_reply_markers(content, markers);
    let content = normalized.as_str();
    let blocks = reply_blocks(content);
    if blocks.iter().any(|block| !block.closed) {
        return ObjectExtraction::NoneFound;
    }
    let array_wrapped = blocks
        .iter()
        .filter(|block| block.info == "json")
        .any(|block| {
            serde_json::from_str::<Value>(block.content.trim())
                .ok()
                .is_some_and(|value| value.is_array())
        })
        || blocks
            .iter()
            .find(|block| block.info == "check")
            .and_then(|check| serde_json::from_str::<Value>(content[..check.start].trim()).ok())
            .is_some_and(|value| value.is_array())
        || serde_json::from_str::<Value>(content.trim())
            .ok()
            .is_some_and(|value| value.is_array());
    let json: Vec<String> = blocks
        .iter()
        .filter(|block| block.info == "json")
        .map(|block| object_text(&block.content))
        .collect();
    let mut bytes = content.as_bytes().to_vec();
    for block in &blocks {
        bytes[block.start..block.end].fill(b' ');
    }
    let source = String::from_utf8(bytes).expect("mask preserves UTF-8");
    let mut depth = 0usize;
    let mut first = None;
    let mut quoted = false;
    let mut escaped = false;
    let mut bare = Vec::new();
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
                bare.push(source[first.unwrap()..=index].to_string());
            }
        }
    }
    let candidates: Vec<String> = json.into_iter().chain(bare).collect();
    let parsed: Vec<_> = candidates
        .iter()
        .map(|candidate| serde_json::from_str::<Value>(candidate))
        .collect();
    if parsed.iter().any(Result::is_err) {
        return if candidates.len() == 1 {
            if array_wrapped {
                ObjectExtraction::ArrayWrappedEntry(candidates[0].clone())
            } else {
                ObjectExtraction::Entry(candidates[0].clone())
            }
        } else {
            ObjectExtraction::SeveralDifferentEntries {
                count: candidates.len(),
                array_wrapped,
            }
        };
    }
    let parsed: Vec<Value> = parsed.into_iter().map(Result::unwrap).collect();
    if !parsed.is_empty() {
        if parsed
            .first()
            .is_some_and(|first| parsed.iter().all(|value| value == first))
        {
            if array_wrapped {
                ObjectExtraction::ArrayWrappedEntry(candidates[0].clone())
            } else {
                ObjectExtraction::Entry(candidates[0].clone())
            }
        } else {
            ObjectExtraction::SeveralDifferentEntries {
                count: parsed
                    .iter()
                    .enumerate()
                    .filter(|(index, value)| !parsed[..*index].contains(value))
                    .count(),
                array_wrapped,
            }
        }
    } else if let Some(entry) = candidates.first() {
        if array_wrapped {
            ObjectExtraction::ArrayWrappedEntry(entry.clone())
        } else {
            ObjectExtraction::Entry(entry.clone())
        }
    } else {
        ObjectExtraction::NoneFound
    }
}

fn object_text(body: &str) -> String {
    let trimmed = body.trim();
    if trimmed.starts_with('[')
        && let Ok(Value::Array(items)) = serde_json::from_str::<Value>(trimmed)
    {
        if items.len() > 1 {
            return String::new();
        }
        if let Some(item) = items.first() {
            return item.to_string();
        }
    }
    match (body.find('{'), body.rfind('}')) {
        (Some(first), Some(last)) if last > first => body[first..=last].to_string(),
        _ => body.trim().to_string(),
    }
}
