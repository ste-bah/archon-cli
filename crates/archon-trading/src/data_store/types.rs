use super::*;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StoredDatasetRecord {
    pub dataset_id: String,
    pub version: String,
    #[serde(default = "registry_contract_schema")]
    #[serde(rename = "schema", alias = "schema_version")]
    pub schema_version: String,
    #[serde(default)]
    pub dataset_path: String,
    #[serde(default)]
    pub metadata_checksum: String,
    #[serde(default)]
    pub raw_checksum: String,
    #[serde(default)]
    pub validation_checksum: String,
    #[serde(default)]
    pub raw_response_path: String,
    #[serde(default)]
    pub raw_request_path: String,
    #[serde(default)]
    pub redacted_headers_path: String,
    #[serde(default)]
    pub provider_notes_path: String,
    pub provider: String,
    pub data_type: String,
    #[serde(default)]
    pub symbol: String,
    #[serde(default)]
    pub timeframe: String,
    #[serde(default)]
    pub native_interval: bool,
    #[serde(default)]
    pub production_eligible: bool,
    pub status: DatasetStatus,
    pub checksum: String,
    pub bars: usize,
    pub coverage_start: String,
    pub coverage_end: String,
    pub metadata_path: String,
    pub normalized_path: String,
    pub raw_path: String,
    #[serde(default)]
    pub validation_path: String,
    #[serde(default)]
    pub manifest_path: String,
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
    #[serde(rename = "schema", alias = "schema_version")]
    pub schema_version: String,
    pub datasets: BTreeMap<String, StoredDatasetRecord>,
    #[serde(default)]
    pub snapshots: BTreeMap<String, serde_json::Value>,
    #[serde(default)]
    pub last_updated: String,
}

impl Default for PersistentDatasetRegistry {
    fn default() -> Self {
        Self {
            schema_version: REGISTRY_SCHEMA_V2.into(),
            datasets: BTreeMap::new(),
            snapshots: BTreeMap::new(),
            last_updated: String::new(),
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
    InvalidMetadata(String),
    IncompleteArtifactContract(String),
    InvalidRegistrySchema(String),
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RegistryMigrationReport {
    #[serde(rename = "schema", alias = "schema_version")]
    pub schema_version: String,
    pub migrated: usize,
    pub skipped: usize,
    pub degraded: usize,
    pub failed: usize,
    pub backup_path: Option<String>,
    #[serde(default)]
    pub report_path: Option<String>,
    #[serde(default)]
    pub validation_report_paths: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct StoreOhlcvRequest {
    pub metadata: DatasetMetadata,
    pub bars: Vec<OhlcvBar>,
    pub raw_body: Vec<u8>,
    pub raw_format: OhlcvFormat,
    pub raw_request: serde_json::Value,
    pub redacted_headers: serde_json::Value,
    pub provider_notes: String,
    pub created_at: String,
}

#[derive(Debug, Clone)]
pub struct TradingDataLake {
    pub(super) root: PathBuf,
}
