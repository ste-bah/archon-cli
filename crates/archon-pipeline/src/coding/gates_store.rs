use super::*;

/// Save a gate result to `<session_dir>/gate-results/<gate_name>.json`.
pub fn save_gate_result(result: &GateResultRecord, session_dir: &Path) -> Result<()> {
    let dir = session_dir.join("gate-results");
    std::fs::create_dir_all(&dir)?;
    let path = dir.join(format!("{}.json", result.gate_name));
    let json = serde_json::to_string_pretty(result)?;
    std::fs::write(&path, json)?;
    Ok(())
}

/// Load a gate result from `<session_dir>/gate-results/<gate_name>.json`.
pub fn load_gate_result(gate_name: &str, session_dir: &Path) -> Result<GateResultRecord> {
    let path = session_dir
        .join("gate-results")
        .join(format!("{}.json", gate_name));
    let data = std::fs::read_to_string(&path)?;
    let result: GateResultRecord = serde_json::from_str(&data)?;
    Ok(result)
}
