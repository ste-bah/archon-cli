//! Rewrite structured input as data, then quote the complete argument again.
use serde_json::Value;

pub(super) fn rewrite(text: &str, root: &str) -> Option<String> {
    let mut result = String::new();
    let mut copied = 0;
    let mut start = None;
    let mut word = String::new();
    let mut quote = None;
    let mut escaped = false;
    let mut found = false;
    for (offset, ch) in text
        .char_indices()
        .chain(std::iter::once((text.len(), ' ')))
    {
        if start.is_none() {
            if ch.is_whitespace() {
                continue;
            }
            start = Some(offset);
        }
        if escaped {
            word.push(ch);
            escaped = false;
        } else if ch == '\\' && quote != Some('\'') {
            escaped = true;
        } else if quote == Some(ch) {
            quote = None;
        } else if quote.is_none() && matches!(ch, '\'' | '"') {
            quote = Some(ch);
        } else if quote.is_none() && ch.is_whitespace() {
            if let Ok(mut value @ Value::Object(_)) = serde_json::from_str::<Value>(&word) {
                rewrite_data(&mut value, &root.replace('\\', "/"));
                let argument = value.to_string();
                result.push_str(&text[copied..start.unwrap()]);
                result.push_str(&format!("'{}'", argument.replace('\'', "'\"'\"'")));
                copied = offset;
                found = true;
            }
            start = None;
            word.clear();
        } else {
            word.push(ch);
        }
    }
    if !found || quote.is_some() || escaped {
        return None;
    }
    result.push_str(&text[copied..]);
    Some(result)
}

fn rewrite_data(value: &mut Value, root: &str) {
    match value {
        Value::String(text) => {
            if let Some((_, absolute)) = super::resolve_project_path(root, text) {
                *text = absolute;
            }
        }
        Value::Array(values) => values
            .iter_mut()
            .for_each(|value| rewrite_data(value, root)),
        Value::Object(values) => values
            .values_mut()
            .for_each(|value| rewrite_data(value, root)),
        _ => {}
    }
}
