//! Volatile fields in compact JSON output.

/// Normalize volatile JSON values without depending on property order.
pub(super) fn normalise_compact_json(token: &str) -> Option<String> {
    if !token.starts_with('{') || !token.ends_with('}') {
        return None;
    }
    let bytes = token.as_bytes();
    let mut output = String::with_capacity(token.len());
    let mut cursor = 0;
    let mut changed = false;
    while cursor < bytes.len() {
        if bytes[cursor] != b'"' {
            let character = token[cursor..].chars().next()?;
            output.push(character);
            cursor += character.len_utf8();
            continue;
        }
        let key_start = cursor + 1;
        let Some(key_end) = string_end(bytes, key_start) else {
            return None;
        };
        let key = &token[key_start..key_end];
        let mut value_start = key_end + 1;
        while bytes.get(value_start).is_some_and(u8::is_ascii_whitespace) {
            value_start += 1;
        }
        if bytes.get(value_start) != Some(&b':') {
            output.push_str(&token[cursor..key_end + 1]);
            cursor = key_end + 1;
            continue;
        }
        value_start += 1;
        while bytes.get(value_start).is_some_and(u8::is_ascii_whitespace) {
            value_start += 1;
        }
        let request_id = super::is_request_id_key(key);
        let Some(value_end) = (bytes.get(value_start) == Some(&b'"'))
            .then(|| string_end(bytes, value_start + 1))
            .flatten()
        else {
            output.push_str(&token[cursor..value_start]);
            cursor = value_start;
            continue;
        };
        let value = &token[value_start + 1..value_end];
        let timestamp = matches!(key.to_ascii_lowercase().as_str(), "ts" | "timestamp")
            && super::is_timestamp(value);
        let scratch = (!request_id && !timestamp)
            .then(|| normalise_scratch_paths(value))
            .flatten();
        if request_id || timestamp || scratch.is_some() {
            output.push_str(&token[cursor..value_start + 1]);
            if request_id {
                output.push_str("<request-id>");
            } else if timestamp {
                output.push_str("<volatile>");
            } else {
                output.push_str(scratch.as_deref().unwrap_or(value));
            }
            output.push('"');
            cursor = value_end + 1;
            changed = true;
        } else {
            output.push_str(&token[cursor..value_end + 1]);
            cursor = value_end + 1;
        }
    }
    changed.then_some(output)
}

fn normalise_scratch_paths(value: &str) -> Option<String> {
    let mut output = String::with_capacity(value.len());
    let mut cursor = 0;
    let mut changed = false;
    while cursor < value.len() {
        let next = super::TEMP_MARKERS
            .iter()
            .filter_map(|marker| value[cursor..].find(marker).map(|offset| (offset, *marker)))
            .min_by_key(|(offset, _)| *offset);
        let Some((offset, marker)) = next else {
            output.push_str(&value[cursor..]);
            break;
        };
        let start = cursor + offset;
        output.push_str(&value[cursor..start]);
        let path_start = start + marker.len();
        let path_end = value[path_start..]
            .find(|ch: char| ch.is_whitespace() || matches!(ch, ':' | ';' | ',' | ')' | ']'))
            .map_or(value.len(), |offset| path_start + offset);
        output.push_str("<scratch>");
        cursor = path_end;
        changed = true;
    }
    changed.then_some(output)
}

fn string_end(bytes: &[u8], mut cursor: usize) -> Option<usize> {
    while let Some(byte) = bytes.get(cursor) {
        match byte {
            b'\\' => cursor += 2,
            b'"' => return Some(cursor),
            _ => cursor += 1,
        }
    }
    None
}
