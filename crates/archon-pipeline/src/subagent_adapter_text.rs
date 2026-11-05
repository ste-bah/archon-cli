pub(super) fn values_to_text(values: &[serde_json::Value]) -> String {
    values
        .iter()
        .map(value_to_text)
        .filter(|text| !text.trim().is_empty())
        .collect::<Vec<_>>()
        .join("\n\n")
}

pub(super) fn value_to_text(value: &serde_json::Value) -> String {
    if let Some(text) = value.get("text").and_then(|v| v.as_str()) {
        return text.to_string();
    }
    if let Some(content) = value.get("content") {
        if let Some(text) = content.as_str() {
            return text.to_string();
        }
        if let Some(parts) = content.as_array() {
            return values_to_text(parts);
        }
    }
    serde_json::to_string_pretty(value).unwrap_or_else(|_| value.to_string())
}
