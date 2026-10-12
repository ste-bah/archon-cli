use serde_json::Value;

pub(crate) struct ReplyBlock {
    pub(crate) info: String,
    pub(crate) content: String,
    pub(crate) start: usize,
    pub(crate) end: usize,
    pub(crate) opened_line: usize,
    pub(crate) closed: bool,
}

pub(crate) fn reply_blocks(content: &str) -> Vec<ReplyBlock> {
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

pub(crate) fn unclosed_block_refusal(content: &str) -> Option<String> {
    reply_blocks(content)
        .into_iter()
        .find(|block| !block.closed)
        .map(|block| {
            format!(
                "the ```{} block opened at line {} is never closed (the reply may have been cut off); close every block with a line that is exactly ```",
                block.info, block.opened_line
            )
        })
}

/// Resolve one referenced block before seed validation and stamping.
pub(crate) fn resolve_command_block(entry: &mut Value, content: &str) -> Option<String> {
    let id = entry
        .get("id")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
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
            "acceptance entry {id} returned both check.command and check.command_block"
        ));
    }
    if command_block == Some(&Value::Bool(true)) && checks.is_empty() {
        return Some(format!(
            "acceptance entry {id} check.command_block is true but no check block is present"
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
    check.insert("command".into(), Value::String(block.content.clone()));
    check.remove("command_block");
    None
}
/// The JSON object a reply carries: an explicit json fence, or the first
/// top-level object outside any check block.
pub(crate) fn extract_object(content: &str) -> String {
    let blocks = reply_blocks(content);
    if blocks.iter().any(|block| !block.closed) {
        return String::new();
    }
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
            candidates[0].clone()
        } else {
            String::new()
        };
    }
    let parsed: Vec<Value> = parsed.into_iter().map(Result::unwrap).collect();
    if !parsed.is_empty() {
        return if parsed
            .first()
            .is_some_and(|first| parsed.iter().all(|value| value == first))
        {
            candidates[0].clone()
        } else {
            String::new()
        };
    }
    candidates.first().cloned().unwrap_or_default()
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
