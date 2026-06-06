use crate::data_lake::{DatasetMetadata, DatasetStatus, VersionedDataset, status_from_gaps};
use crate::ohlcv::{OhlcvBar, OhlcvFormat, bars_checksum, coverage_bounds, validate_bars};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

const REGISTRY_SCHEMA: &str = "archon-trading-data-registry-v1";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StoredDatasetRecord {
    pub dataset_id: String,
    pub version: String,
    pub provider: String,
    pub data_type: String,
    pub status: DatasetStatus,
    pub checksum: String,
    pub bars: usize,
    pub coverage_start: String,
    pub coverage_end: String,
    pub metadata_path: String,
    pub normalized_path: String,
    pub raw_path: String,
    pub created_at: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StoredOhlcvDataset {
    pub record: StoredDatasetRecord,
    pub metadata: DatasetMetadata,
    pub bars: Vec<OhlcvBar>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PersistentDatasetRegistry {
    pub schema_version: String,
    pub datasets: BTreeMap<String, StoredDatasetRecord>,
}

impl Default for PersistentDatasetRegistry {
    fn default() -> Self {
        Self {
            schema_version: REGISTRY_SCHEMA.into(),
            datasets: BTreeMap::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DataStoreError {
    MissingDataset(String),
    InvalidPath,
    InvalidOhlcv(String),
    Io(String),
    Json(String),
}

#[derive(Debug, Clone)]
pub struct StoreOhlcvRequest {
    pub metadata: DatasetMetadata,
    pub bars: Vec<OhlcvBar>,
    pub raw_body: Vec<u8>,
    pub raw_format: OhlcvFormat,
    pub created_at: String,
}

#[derive(Debug, Clone)]
pub struct TradingDataLake {
    root: PathBuf,
}

impl TradingDataLake {
    pub fn new(project_root: impl Into<PathBuf>) -> Self {
        Self {
            root: project_root.into(),
        }
    }

    pub fn data_root(&self) -> PathBuf {
        self.root.join(".archon/trading-lab/data")
    }

    pub fn registry_path(&self) -> PathBuf {
        self.data_root().join("registry.json")
    }

    pub fn status(&self) -> Result<PersistentDatasetRegistry, DataStoreError> {
        self.load_registry()
    }

    pub fn store_ohlcv(
        &self,
        request: StoreOhlcvRequest,
    ) -> Result<StoredDatasetRecord, DataStoreError> {
        validate_bars(&request.bars)
            .map_err(|err| DataStoreError::InvalidOhlcv(format!("{err:?}")))?;
        let mut metadata = request.metadata;
        metadata.checksum = bars_checksum(&request.bars);
        metadata.coverage.observed_bars = request.bars.len() as u64;
        if metadata.coverage.expected_bars == 0 {
            metadata.coverage.expected_bars = request.bars.len() as u64;
        }
        if metadata.gaps.expected_bars == 0 {
            metadata.gaps.expected_bars = metadata.coverage.expected_bars;
        }
        let Some((start, end)) = coverage_bounds(&request.bars) else {
            return Err(DataStoreError::InvalidOhlcv("empty".into()));
        };
        metadata.coverage.start = start;
        metadata.coverage.end = end;
        let versioned = VersionedDataset {
            content_hash: metadata.checksum.clone(),
            status: status_from_gaps(&metadata.gaps),
            metadata,
        };
        self.write_dataset(
            versioned,
            request.bars,
            request.raw_body,
            request.raw_format,
            request.created_at,
        )
    }

    pub fn load_ohlcv(
        &self,
        dataset_id: &str,
        version: &str,
    ) -> Result<StoredOhlcvDataset, DataStoreError> {
        let registry = self.load_registry()?;
        let record = registry
            .datasets
            .get(&registry_key(dataset_id, version))
            .cloned()
            .ok_or_else(|| DataStoreError::MissingDataset(registry_key(dataset_id, version)))?;
        let metadata: DatasetMetadata = read_json(&self.root.join(&record.metadata_path))?;
        let bars = read_jsonl_bars(&self.root.join(&record.normalized_path))?;
        Ok(StoredOhlcvDataset {
            record,
            metadata,
            bars,
        })
    }

    fn write_dataset(
        &self,
        versioned: VersionedDataset,
        bars: Vec<OhlcvBar>,
        raw_body: Vec<u8>,
        raw_format: OhlcvFormat,
        created_at: String,
    ) -> Result<StoredDatasetRecord, DataStoreError> {
        let dir = self.dataset_dir(&versioned.metadata.dataset_id, &versioned.metadata.version);
        std::fs::create_dir_all(&dir).map_err(io_error)?;
        let raw_path = dir.join(raw_filename(raw_format));
        let metadata_path = dir.join("metadata.json");
        let normalized_path = dir.join("ohlcv.jsonl");
        std::fs::write(&raw_path, raw_body).map_err(io_error)?;
        write_json(&metadata_path, &versioned.metadata)?;
        write_jsonl_bars(&normalized_path, &bars)?;
        let record = record(
            &self.root,
            &versioned,
            &bars,
            &metadata_path,
            &normalized_path,
            &raw_path,
            created_at,
        )?;
        let mut registry = self.load_registry()?;
        registry.datasets.insert(
            registry_key(&record.dataset_id, &record.version),
            record.clone(),
        );
        write_json(&self.registry_path(), &registry)?;
        Ok(record)
    }

    fn load_registry(&self) -> Result<PersistentDatasetRegistry, DataStoreError> {
        let path = self.registry_path();
        if !path.exists() {
            return Ok(PersistentDatasetRegistry::default());
        }
        read_json(&path)
    }

    fn dataset_dir(&self, dataset_id: &str, version: &str) -> PathBuf {
        self.data_root()
            .join("datasets")
            .join(safe_path(dataset_id))
            .join(safe_path(version))
    }
}

fn record(
    root: &Path,
    versioned: &VersionedDataset,
    bars: &[OhlcvBar],
    metadata_path: &Path,
    normalized_path: &Path,
    raw_path: &Path,
    created_at: String,
) -> Result<StoredDatasetRecord, DataStoreError> {
    Ok(StoredDatasetRecord {
        dataset_id: versioned.metadata.dataset_id.clone(),
        version: versioned.metadata.version.clone(),
        provider: versioned.metadata.provider.clone(),
        data_type: format!("{:?}", versioned.metadata.data_type),
        status: versioned.status,
        checksum: versioned.metadata.checksum.clone(),
        bars: bars.len(),
        coverage_start: versioned.metadata.coverage.start.clone(),
        coverage_end: versioned.metadata.coverage.end.clone(),
        metadata_path: relative(root, metadata_path)?,
        normalized_path: relative(root, normalized_path)?,
        raw_path: relative(root, raw_path)?,
        created_at,
    })
}

fn read_json<T: for<'de> Deserialize<'de>>(path: &Path) -> Result<T, DataStoreError> {
    let text = std::fs::read_to_string(path).map_err(io_error)?;
    serde_json::from_str(&text).map_err(|err| DataStoreError::Json(err.to_string()))
}

fn write_json<T: Serialize>(path: &Path, value: &T) -> Result<(), DataStoreError> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(io_error)?;
    }
    let text =
        serde_json::to_string_pretty(value).map_err(|err| DataStoreError::Json(err.to_string()))?;
    std::fs::write(path, text).map_err(io_error)
}

fn read_jsonl_bars(path: &Path) -> Result<Vec<OhlcvBar>, DataStoreError> {
    let text = std::fs::read_to_string(path).map_err(io_error)?;
    text.lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| serde_json::from_str(line).map_err(|err| DataStoreError::Json(err.to_string())))
        .collect()
}

fn write_jsonl_bars(path: &Path, bars: &[OhlcvBar]) -> Result<(), DataStoreError> {
    let mut text = String::new();
    for bar in bars {
        text.push_str(
            &serde_json::to_string(bar).map_err(|err| DataStoreError::Json(err.to_string()))?,
        );
        text.push('\n');
    }
    std::fs::write(path, text).map_err(io_error)
}

fn registry_key(dataset_id: &str, version: &str) -> String {
    format!("{dataset_id}:{version}")
}

fn raw_filename(format: OhlcvFormat) -> &'static str {
    match format {
        OhlcvFormat::Csv => "raw.csv",
        OhlcvFormat::Json => "raw.json",
    }
}

fn safe_path(value: &str) -> String {
    value
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.') {
                c
            } else {
                '_'
            }
        })
        .collect()
}

fn relative(root: &Path, path: &Path) -> Result<String, DataStoreError> {
    path.strip_prefix(root)
        .map(|path| path.to_string_lossy().to_string())
        .map_err(|_| DataStoreError::InvalidPath)
}

fn io_error(error: std::io::Error) -> DataStoreError {
    DataStoreError::Io(error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data_lake::{CoverageWindow, DataType, GapSummary};

    #[test]
    fn stores_and_loads_ohlcv_dataset() {
        let temp = tempfile::tempdir().unwrap();
        let lake = TradingDataLake::new(temp.path());
        let record = lake.store_ohlcv(request()).unwrap();
        assert_eq!(record.bars, 2);
        let loaded = lake.load_ohlcv("btc-1d", "v1").unwrap();
        assert_eq!(loaded.bars.len(), 2);
        assert!(lake.registry_path().exists());
    }

    fn request() -> StoreOhlcvRequest {
        StoreOhlcvRequest {
            metadata: DatasetMetadata {
                dataset_id: "btc-1d".into(),
                provider: "manual".into(),
                data_type: DataType::Ohlcv,
                symbol_map: BTreeMap::from([("BTCUSD".into(), "BTCUSD".into())]),
                timezone: "UTC".into(),
                adjustment: "raw".into(),
                license: "research".into(),
                coverage: CoverageWindow {
                    start: String::new(),
                    end: String::new(),
                    expected_bars: 2,
                    observed_bars: 0,
                },
                gaps: GapSummary {
                    missing_bars: 0,
                    expected_bars: 2,
                },
                checksum: String::new(),
                version: "v1".into(),
                optional: false,
            },
            bars: vec![bar("2026-01-01", 10.0), bar("2026-01-02", 11.0)],
            raw_body: b"raw".to_vec(),
            raw_format: OhlcvFormat::Csv,
            created_at: "2026-01-01T00:00:00Z".into(),
        }
    }

    fn bar(timestamp: &str, close: f64) -> OhlcvBar {
        OhlcvBar {
            timestamp: timestamp.into(),
            open: close,
            high: close + 1.0,
            low: close - 1.0,
            close,
            volume: 1.0,
        }
    }
}
