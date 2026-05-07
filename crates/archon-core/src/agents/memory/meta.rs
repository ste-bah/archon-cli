use std::path::Path;

/// Increment the invocation_count in an agent's meta.json.
///
/// Returns the new count, or an error message.
pub fn increment_invocation_count(agent_dir: &Path) -> Result<u64, String> {
    let meta_path = agent_dir.join("meta.json");
    let content = std::fs::read_to_string(&meta_path)
        .map_err(|e| format!("failed to read meta.json: {e}"))?;

    let mut meta: serde_json::Value =
        serde_json::from_str(&content).map_err(|e| format!("failed to parse meta.json: {e}"))?;

    let count = meta
        .get("invocation_count")
        .and_then(|v| v.as_u64())
        .unwrap_or(0)
        + 1;

    meta["invocation_count"] = serde_json::json!(count);
    meta["updated_at"] = serde_json::json!(chrono::Utc::now().to_rfc3339());

    let serialized = serde_json::to_string_pretty(&meta)
        .map_err(|e| format!("failed to serialize meta.json: {e}"))?;

    std::fs::write(&meta_path, serialized)
        .map_err(|e| format!("failed to write meta.json: {e}"))?;

    Ok(count)
}
