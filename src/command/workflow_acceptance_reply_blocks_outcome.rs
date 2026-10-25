use super::*;

pub(crate) enum ObjectExtraction {
    Entry(String),
    NoneFound,
    SeveralDifferentEntries {
        count: usize,
        array_wrapped: bool,
    },
    ArrayWrappedEntry(String),
    MisplacedBrace {
        missing_fields: Vec<String>,
        next_field: String,
    },
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
    let mut first_end = None;
    let mut first_bare = None;
    let mut stray_after = None;
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
                let start = first.expect("an open object has a start");
                if first_end.is_none() {
                    first_end = Some(index);
                    first_bare = Some(source[start..=index].to_string());
                }
                bare.push(source[start..=index].to_string());
            }
        } else if byte == b'}' && first_end.is_some_and(|end| index > end) {
            stray_after.get_or_insert(index);
        }
    }
    let candidates: Vec<String> = json.into_iter().chain(bare).collect();
    if let (Some(end), Some(candidate)) = (first_end, first_bare.as_deref())
        && let Some(tail) = source.get(end + 1..)
        && let Some(next_field) = continued_key(tail)
        && stray_after.is_some()
    {
        return ObjectExtraction::MisplacedBrace {
            missing_fields: missing_check_fields(candidate, content, markers).unwrap_or_default(),
            next_field,
        };
    }
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

fn continued_key(tail: &str) -> Option<String> {
    let after_comma = tail.trim_start().strip_prefix(',')?.trim_start();
    let after_quote = after_comma.strip_prefix('"')?;
    let end = after_quote.find('"')?;
    let key = &after_quote[..end];
    after_quote[end + 1..]
        .trim_start()
        .starts_with(':')
        .then(|| key.to_string())
}

fn missing_check_fields(candidate: &str, content: &str, markers: &[String]) -> Option<Vec<String>> {
    let mut value = serde_json::from_str::<Value>(candidate).ok()?;
    let _ = super::resolve_command_block_with_markers(&mut value, content, markers);
    let check = value.get_mut("check")?.as_object_mut()?;
    let mut missing = Vec::new();
    loop {
        match serde_json::from_value::<archon_workflow::task_set_contract::AcceptanceCheck>(
            Value::Object(check.clone()),
        ) {
            Ok(_) => break,
            Err(error) => {
                let error = error.to_string();
                let Some(field) = error
                    .split_once("missing field `")
                    .and_then(|(_, rest)| rest.split_once('`').map(|(field, _)| field))
                else {
                    break;
                };
                missing.push(format!("check.{field}"));
                check.insert(field.to_string(), Value::String(String::new()));
            }
        }
    }
    (!missing.is_empty()).then_some(missing)
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
