use super::*;
pub(super) fn write_jsonl_trades(
    path: &Path,
    trades: &[crate::candle_backtest::OhlcvTrade],
) -> Result<(), DataStoreError> {
    let mut text = String::new();
    for trade in trades {
        text.push_str(
            &serde_json::to_string(trade).map_err(|err| DataStoreError::Json(err.to_string()))?,
        );
        text.push('\n');
    }
    write_text(path, &text)
}

pub(super) fn write_equity_curve(
    path: &Path,
    starting_equity: f64,
    report: &OhlcvBacktestReport,
) -> Result<(), DataStoreError> {
    let mut equity = starting_equity;
    let mut text = String::new();
    for trade in &report.trades {
        equity += trade.net_pnl;
        text.push_str(
            &serde_json::json!({"timestamp": trade.exit_timestamp, "equity": equity}).to_string(),
        );
        text.push('\n');
    }
    write_text(path, &text)
}

pub(super) fn read_jsonl_bars(path: &Path) -> Result<Vec<OhlcvBar>, DataStoreError> {
    let text = std::fs::read_to_string(path).map_err(io_error)?;
    text.lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| serde_json::from_str(line).map_err(|err| DataStoreError::Json(err.to_string())))
        .collect()
}

pub(super) fn write_jsonl_bars(path: &Path, bars: &[OhlcvBar]) -> Result<(), DataStoreError> {
    let mut text = String::new();
    for bar in bars {
        text.push_str(
            &serde_json::to_string(bar).map_err(|err| DataStoreError::Json(err.to_string()))?,
        );
        text.push('\n');
    }
    write_text(path, &text)
}

pub(super) fn normalized_bars_checksum(bars: &[OhlcvBar]) -> Result<String, DataStoreError> {
    let mut text = String::new();
    for bar in bars {
        text.push_str(
            &serde_json::to_string(bar).map_err(|err| DataStoreError::Json(err.to_string()))?,
        );
        text.push('\n');
    }
    Ok(bytes_checksum(text.as_bytes()))
}

pub(super) fn read_json<T: for<'de> Deserialize<'de>>(path: &Path) -> Result<T, DataStoreError> {
    let text = std::fs::read_to_string(path).map_err(io_error)?;
    let normalized = strip_legacy_duplicate_schema_version(&text)?;
    serde_json::from_str(&normalized).map_err(|err| DataStoreError::Json(err.to_string()))
}

fn strip_legacy_duplicate_schema_version(text: &str) -> Result<String, DataStoreError> {
    let mut value: serde_json::Value =
        serde_json::from_str(text).map_err(|err| DataStoreError::Json(err.to_string()))?;
    if let Some(object) = value.as_object_mut()
        && object.contains_key("schema")
        && object.contains_key("schema_version")
    {
        object.remove("schema_version");
    }
    if let Some(datasets) = value
        .get_mut("datasets")
        .and_then(serde_json::Value::as_object_mut)
    {
        for record in datasets.values_mut() {
            if let Some(object) = record.as_object_mut() {
                if object.contains_key("schema") && object.contains_key("schema_version") {
                    object.remove("schema_version");
                }
                if object.get("status").and_then(serde_json::Value::as_str) == Some("Available") {
                    object.insert("status".into(), serde_json::Value::String("Healthy".into()));
                }
            }
        }
    }
    serde_json::to_string(&value).map_err(|err| DataStoreError::Json(err.to_string()))
}

pub(super) fn write_json<T: Serialize>(path: &Path, value: &T) -> Result<(), DataStoreError> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(io_error)?;
    }
    let text =
        serde_json::to_string_pretty(value).map_err(|err| DataStoreError::Json(err.to_string()))?;
    let temp_path = path.with_extension("tmp");
    atomic_write(path, &temp_path, text.as_bytes())
}

pub(super) fn write_json_with_backup<T: Serialize>(
    path: &Path,
    value: &T,
    backup_path: &Path,
) -> Result<(), DataStoreError> {
    let text =
        serde_json::to_string_pretty(value).map_err(|err| DataStoreError::Json(err.to_string()))?;
    atomic_write_with_backup(
        path,
        path.with_extension("tmp"),
        backup_path,
        text.as_bytes(),
    )
}

pub(super) fn write_bytes(path: &Path, bytes: &[u8]) -> Result<(), DataStoreError> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(io_error)?;
    }
    let temp_path = path.with_extension("tmp");
    atomic_write(path, &temp_path, bytes)
}

pub(super) fn write_text(path: &Path, text: &str) -> Result<(), DataStoreError> {
    let temp_path = path.with_extension("tmp");
    atomic_write(path, &temp_path, text.as_bytes())
}

fn atomic_write(path: &Path, temp_path: &Path, bytes: &[u8]) -> Result<(), DataStoreError> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(io_error)?;
    }
    std::fs::write(temp_path, bytes).map_err(io_error)?;
    std::fs::rename(temp_path, path).map_err(io_error)
}

fn atomic_write_with_backup(
    path: &Path,
    temp_path: PathBuf,
    backup_path: &Path,
    bytes: &[u8],
) -> Result<(), DataStoreError> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(io_error)?;
    }
    std::fs::write(&temp_path, bytes).map_err(io_error)?;
    if path.exists() {
        if let Some(parent) = backup_path.parent() {
            std::fs::create_dir_all(parent).map_err(io_error)?;
        }
        std::fs::copy(path, backup_path).map_err(io_error)?;
    }
    std::fs::rename(temp_path, path).map_err(io_error)
}
