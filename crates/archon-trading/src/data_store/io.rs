use super::*;

#[cfg(test)]
thread_local! {
    static FAIL_BOUNDARY: std::cell::RefCell<Option<String>> = const { std::cell::RefCell::new(None) };
}

#[cfg(test)]
pub(super) fn inject_io_failure(boundary: Option<&str>) {
    FAIL_BOUNDARY.with(|value| *value.borrow_mut() = boundary.map(str::to_owned));
}

fn fail_boundary(_boundary: &str) -> Result<(), DataStoreError> {
    #[cfg(test)]
    {
        let injected = FAIL_BOUNDARY.with(|value| {
            let mut value = value.borrow_mut();
            if value.as_deref() == Some(_boundary) {
                value.take();
                true
            } else {
                false
            }
        });
        if injected {
            return Err(DataStoreError::Io(format!(
                "deterministic failure at {_boundary}"
            )));
        }
    }
    Ok(())
}
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
    if text.is_empty() {
        return Err(required_jsonl_fields_error(1));
    }
    text.lines()
        .enumerate()
        .map(|(index, line)| parse_required_jsonl_bar(line, index + 1))
        .collect()
}

fn parse_required_jsonl_bar(line: &str, line_number: usize) -> Result<OhlcvBar, DataStoreError> {
    if line.trim().is_empty() {
        return Err(required_jsonl_fields_error(line_number));
    }
    let mut deserializer = serde_json::Deserializer::from_str(line);
    let parsed: StrictOhlcvBar = serde::Deserialize::deserialize(&mut deserializer)
        .map_err(|_| required_jsonl_fields_error(line_number))?;
    deserializer
        .end()
        .map_err(|_| required_jsonl_fields_error(line_number))?;
    Ok(parsed.0)
}

fn required_jsonl_fields_error(line_number: usize) -> DataStoreError {
    DataStoreError::InvalidOhlcv(format!(
        "ohlcv.required_fields failed at line {line_number}"
    ))
}

struct StrictOhlcvBar(OhlcvBar);

impl<'de> serde::Deserialize<'de> for StrictOhlcvBar {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        deserializer.deserialize_map(StrictOhlcvVisitor)
    }
}

struct StrictOhlcvVisitor;

impl<'de> serde::de::Visitor<'de> for StrictOhlcvVisitor {
    type Value = StrictOhlcvBar;

    fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("an OHLCV object with unique, correctly typed required fields")
    }

    fn visit_map<A>(self, mut map: A) -> Result<Self::Value, A::Error>
    where
        A: serde::de::MapAccess<'de>,
    {
        let mut fields = StrictOhlcvFields::default();
        let mut seen = std::collections::BTreeSet::new();
        while let Some(key) = map.next_key::<String>()? {
            let canonical = if key == "ts" {
                "timestamp"
            } else {
                key.as_str()
            };
            if !seen.insert(canonical.to_owned()) {
                return Err(serde::de::Error::custom("duplicate OHLCV field"));
            }
            match canonical {
                "timestamp" => fields.timestamp = Some(map.next_value::<String>()?),
                "open" => fields.open = Some(map.next_value::<f64>()?),
                "high" => fields.high = Some(map.next_value::<f64>()?),
                "low" => fields.low = Some(map.next_value::<f64>()?),
                "close" => fields.close = Some(map.next_value::<f64>()?),
                "volume" => fields.volume = Some(map.next_value::<f64>()?),
                _ => {
                    map.next_value::<serde::de::IgnoredAny>()?;
                }
            }
        }
        fields.into_bar().map(StrictOhlcvBar)
    }
}

#[derive(Default)]
struct StrictOhlcvFields {
    timestamp: Option<String>,
    open: Option<f64>,
    high: Option<f64>,
    low: Option<f64>,
    close: Option<f64>,
    volume: Option<f64>,
}

impl StrictOhlcvFields {
    fn into_bar<E: serde::de::Error>(self) -> Result<OhlcvBar, E> {
        Ok(OhlcvBar {
            timestamp: self
                .timestamp
                .ok_or_else(|| E::custom("missing timestamp"))?,
            open: self.open.ok_or_else(|| E::custom("missing open"))?,
            high: self.high.ok_or_else(|| E::custom("missing high"))?,
            low: self.low.ok_or_else(|| E::custom("missing low"))?,
            close: self.close.ok_or_else(|| E::custom("missing close"))?,
            volume: self.volume.ok_or_else(|| E::custom("missing volume"))?,
        })
    }
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
    let previous = path
        .exists()
        .then(|| std::fs::read(path))
        .transpose()
        .map_err(io_error)?;
    if let Err(error) = write_synced(temp_path, bytes) {
        let _ = std::fs::remove_file(temp_path);
        return Err(error);
    }
    let commit = fail_boundary("file.rename")
        .and_then(|()| std::fs::rename(temp_path, path).map_err(io_error))
        .and_then(|()| sync_parent(path));
    if let Err(error) = commit {
        let _ = std::fs::remove_file(temp_path);
        restore_previous(path, previous.as_deref())?;
        return Err(error);
    }
    Ok(())
}

fn restore_previous(path: &Path, previous: Option<&[u8]>) -> Result<(), DataStoreError> {
    match previous {
        Some(bytes) => std::fs::write(path, bytes).map_err(io_error)?,
        None if path.exists() => std::fs::remove_file(path).map_err(io_error)?,
        None => {}
    }
    sync_parent(path)
}

fn write_synced(path: &Path, bytes: &[u8]) -> Result<(), DataStoreError> {
    use std::io::Write;
    fail_boundary("file.create")?;
    let mut file = std::fs::File::create(path).map_err(io_error)?;
    fail_boundary("file.write")?;
    file.write_all(bytes).map_err(io_error)?;
    fail_boundary("file.sync")?;
    file.sync_all().map_err(io_error)
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
    write_synced(&temp_path, bytes)?;
    let previous = if path.exists() {
        Some(std::fs::read(path).map_err(io_error)?)
    } else {
        None
    };
    if let Some(previous) = previous.as_ref() {
        if let Some(parent) = backup_path.parent() {
            std::fs::create_dir_all(parent).map_err(io_error)?;
        }
        write_synced(backup_path, previous)?;
        sync_parent(backup_path)?;
    }
    if let Err(error) = fail_boundary("file.rename")
        .and_then(|()| std::fs::rename(&temp_path, path).map_err(io_error))
    {
        if let Some(previous) = previous {
            std::fs::write(path, previous).map_err(io_error)?;
        }
        return Err(error);
    }
    if let Err(error) = sync_parent(path) {
        restore_previous(path, previous.as_deref())?;
        return Err(error);
    }
    Ok(())
}

pub(super) fn atomic_write_many(updates: Vec<(PathBuf, Vec<u8>)>) -> Result<(), DataStoreError> {
    let staged = stage_updates(&updates)?;
    let previous = updates
        .iter()
        .map(|(path, _)| {
            if path.exists() {
                std::fs::read(path).map(Some).map_err(io_error)
            } else {
                Ok(None)
            }
        })
        .collect::<Result<Vec<_>, _>>()?;

    for (index, ((path, _), temp)) in updates.iter().zip(&staged).enumerate() {
        if let Err(error) = fail_transaction_boundary("replace", index, "transaction.rename")
            .and_then(|()| std::fs::rename(temp, path).map_err(io_error))
        {
            rollback_updates(&updates[..index], &previous[..index])?;
            remove_staged(&staged[index..]);
            return Err(error);
        }
        if let Err(error) = sync_transaction_parent(path, index) {
            rollback_updates(&updates[..=index], &previous[..=index])?;
            remove_staged(&staged[index + 1..]);
            return Err(error);
        }
    }
    Ok(())
}

fn stage_updates(updates: &[(PathBuf, Vec<u8>)]) -> Result<Vec<PathBuf>, DataStoreError> {
    let mut staged = Vec::with_capacity(updates.len());
    for (index, (path, bytes)) in updates.iter().enumerate() {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(io_error)?;
        }
        let temp = path.with_extension(format!("validation-txn-{index}.tmp"));
        if let Err(error) = write_transaction_temp(&temp, bytes, index) {
            let _ = std::fs::remove_file(&temp);
            remove_staged(&staged);
            return Err(error);
        }
        staged.push(temp);
    }
    Ok(staged)
}

fn write_transaction_temp(path: &Path, bytes: &[u8], index: usize) -> Result<(), DataStoreError> {
    use std::io::Write;
    fail_transaction_boundary("temp.create", index, "file.create")?;
    let mut file = std::fs::File::create(path).map_err(io_error)?;
    fail_transaction_boundary("temp.write", index, "file.write")?;
    file.write_all(bytes).map_err(io_error)?;
    fail_boundary(&format!("transaction.temp.flush.{index}"))?;
    file.flush().map_err(io_error)?;
    fail_transaction_boundary("temp.file_sync", index, "file.sync")?;
    file.sync_all().map_err(io_error)
}

fn fail_transaction_boundary(
    operation: &str,
    index: usize,
    legacy_boundary: &str,
) -> Result<(), DataStoreError> {
    fail_boundary(&format!("transaction.{operation}.{index}"))?;
    fail_boundary(legacy_boundary)
}

fn sync_transaction_parent(path: &Path, index: usize) -> Result<(), DataStoreError> {
    fail_boundary(&format!("transaction.directory_sync.{index}"))?;
    sync_parent(path)
}

fn rollback_updates(
    updates: &[(PathBuf, Vec<u8>)],
    previous: &[Option<Vec<u8>>],
) -> Result<(), DataStoreError> {
    for ((path, _), old) in updates.iter().zip(previous) {
        fail_boundary("rollback.write")?;
        match old {
            Some(bytes) => std::fs::write(path, bytes).map_err(io_error)?,
            None if path.exists() => std::fs::remove_file(path).map_err(io_error)?,
            None => {}
        }
        sync_parent(path)?;
    }
    Ok(())
}

fn remove_staged(paths: &[PathBuf]) {
    for path in paths {
        let _ = std::fs::remove_file(path);
    }
}

fn sync_parent(path: &Path) -> Result<(), DataStoreError> {
    fail_boundary("directory.sync")?;
    let parent = path.parent().ok_or(DataStoreError::InvalidPath)?;
    std::fs::File::open(parent)
        .and_then(|directory| directory.sync_all())
        .map_err(io_error)
}
