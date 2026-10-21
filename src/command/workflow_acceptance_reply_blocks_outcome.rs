use super::*;

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
