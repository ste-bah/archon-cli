use crate::backtest::{BacktestConfig, EvidenceSource};
use crate::candle_backtest::{OhlcvBacktestReport, run_ohlcv_backtest};
use crate::data_lake::{
    BacktestDataGateReport, CoverageCell, CoverageGap, CoverageMatrix, DatasetArtifactPaths,
    DatasetChecksums, DatasetMetadata, DatasetSourceMetadata, DatasetStatus,
    ProviderCapabilityResult, SnapshotFreshness, ValidationCheck, ValidationReport,
    ValidationSeverity, ValidationStatus, ValidationSummary, VersionedDataset,
    can_fetch_symbol_timeframe, normalize_timeframe, provider_supports_native_timeframe,
    snapshot_freshness, status_from_metadata, validate_metadata,
};
use crate::ohlcv::{
    OhlcvBacktestRequest, OhlcvBacktestRule, OhlcvBar, OhlcvDatasetRef, OhlcvFormat,
    bytes_checksum, coverage_bounds, validate_bars,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

const REGISTRY_SCHEMA_V1: &str = "archon-trading-data-registry-v1";
const REGISTRY_SCHEMA_V2: &str = "archon-trading-data-registry-v2";

mod ahdm;
mod ahdm_methods;
#[cfg(test)]
mod ahdm_test_support;
mod artifact_schema;
mod coverage;
mod coverage_methods;
mod gates;
mod io;
mod migration;
mod provider_methods;
mod records;
mod util;
mod validation;

use ahdm::*;
use artifact_schema::*;
use coverage::*;
use gates::*;
use io::*;
use migration::*;
use records::*;
use util::*;
use validation::*;

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
    root: PathBuf,
}

impl TradingDataLake {
    pub fn new(project_root: impl Into<PathBuf>) -> Self {
        Self {
            root: project_root.into(),
        }
    }

    pub fn project_root(&self) -> &Path {
        &self.root
    }

    pub fn data_root(&self) -> PathBuf {
        self.root.join(".archon/trading-lab/data")
    }

    pub fn registry_path(&self) -> PathBuf {
        self.data_root().join("registry.json")
    }

    pub fn provider_capabilities_path(&self) -> PathBuf {
        self.data_root().join("provider-capabilities.json")
    }

    pub fn provider_capability_latest_path(&self) -> PathBuf {
        self.data_root()
            .join("provider-capabilities")
            .join("latest.json")
    }

    pub fn coverage_dir(&self) -> PathBuf {
        self.data_root().join("coverage")
    }

    pub fn ahdm_strategy_root(&self) -> PathBuf {
        self.root.join(".archon/trading-lab/strategies/AHDM-v1")
    }

    pub fn snapshot_dir(&self, provider: &str, symbol: &str) -> PathBuf {
        self.data_root()
            .join("snapshots")
            .join(safe_path(provider))
            .join(safe_path(symbol))
    }

    pub fn status(&self) -> Result<PersistentDatasetRegistry, DataStoreError> {
        self.load_verified_registry()
    }

    pub fn migration_report(&self) -> Result<RegistryMigrationReport, DataStoreError> {
        self.load_registry_migration(true)
            .map(|migration| migration.report)
    }

    pub fn store_ohlcv(
        &self,
        request: StoreOhlcvRequest,
    ) -> Result<StoredDatasetRecord, DataStoreError> {
        validate_bars(&request.bars)
            .map_err(|err| DataStoreError::InvalidOhlcv(format!("{err:?}")))?;
        let mut metadata = request.metadata;
        metadata.checksum = normalized_bars_checksum(&request.bars)?;
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
        if contains_secret_material(&request.redacted_headers)
            || contains_secret_material(&request.raw_request)
        {
            return Err(DataStoreError::InvalidMetadata(
                "secret material rejected".into(),
            ));
        }
        fail_closed_non_native_production_metadata(&mut metadata);
        fail_closed_derived_or_resampled_metadata(&mut metadata);
        let versioned = VersionedDataset {
            content_hash: metadata.checksum.clone(),
            status: status_from_metadata(&metadata),
            metadata,
        };
        let registry = self.load_registry_migration(false)?.registry;
        if let Some(existing) = registry.datasets.get(&registry_key(
            &versioned.metadata.dataset_id,
            &versioned.metadata.version,
        )) {
            verify_artifacts(&self.root, existing)?;
            if existing.checksum == versioned.content_hash {
                return Ok(existing.clone());
            }
            return Err(DataStoreError::InvalidMetadata(
                "dataset id/version already exists with different normalized checksum".into(),
            ));
        }
        self.write_dataset(
            versioned,
            request.bars,
            (request.raw_body, request.raw_format),
            (request.raw_request, request.redacted_headers),
            request.provider_notes,
            request.created_at,
        )
    }

    pub fn validate_ohlcv(
        &self,
        dataset_id: &str,
        version: &str,
        validated_at: String,
    ) -> Result<ValidationReport, DataStoreError> {
        let registry = self.load_registry_migration(false)?.registry;
        let record = registry
            .datasets
            .get(&registry_key(dataset_id, version))
            .cloned()
            .ok_or_else(|| DataStoreError::MissingDataset(registry_key(dataset_id, version)))?;
        verify_artifacts(&self.root, &record)?;
        let metadata: DatasetMetadata = read_json(&self.root.join(&record.metadata_path))?;
        let bars = read_jsonl_bars(&self.root.join(&record.normalized_path))?;
        let dataset = StoredOhlcvDataset {
            record,
            metadata,
            bars,
        };
        let report = validation_report(&dataset.metadata, &dataset.bars, validated_at);
        write_schema_json(&self.root.join(&dataset.record.validation_path), &report)?;
        if report.status == ValidationStatus::Failed {
            fail_closed_validation_record(&self.root, &dataset.record, &report)?;
            return Err(DataStoreError::InvalidOhlcv(format!("{report:?}")));
        }
        sync_validation_record(&self.root, &dataset.record, &report)?;
        Ok(report)
    }

    pub fn backtest_data_gate(
        &self,
        dataset_id: &str,
        version: &str,
        diagnostic_allow_degraded_data: bool,
    ) -> Result<BacktestDataGateReport, DataStoreError> {
        let registry = self.load_registry_migration(false)?.registry;
        let key = registry_key(dataset_id, version);
        let record = registry
            .datasets
            .get(&key)
            .cloned()
            .ok_or_else(|| DataStoreError::MissingDataset(key.clone()))?;
        let mut issues = Vec::new();
        append_missing_artifact_issues(&self.root, &record, &mut issues);
        if !issues
            .iter()
            .any(|issue| issue.contains("missing artifact"))
        {
            let dataset = self.load_ohlcv(dataset_id, version)?;
            append_dataset_gate_issues(&self.root, &record, &dataset, &mut issues);
        }
        let report = BacktestDataGateReport {
            dataset_id: dataset_id.into(),
            version: version.into(),
            diagnostic: diagnostic_allow_degraded_data,
            promotion_eligible: issues.is_empty() && !diagnostic_allow_degraded_data,
            overridden_issues: if diagnostic_allow_degraded_data {
                issues.clone()
            } else {
                Vec::new()
            },
            issues,
        };
        if report.issues.is_empty() || diagnostic_allow_degraded_data {
            Ok(report)
        } else {
            Err(DataStoreError::InvalidMetadata(format!(
                "backtest data gate refused dataset {dataset_id}:{version}: {}",
                report.issues.join("; ")
            )))
        }
    }

    pub fn load_ohlcv(
        &self,
        dataset_id: &str,
        version: &str,
    ) -> Result<StoredOhlcvDataset, DataStoreError> {
        let registry = self.load_verified_registry()?;
        let record = registry
            .datasets
            .get(&registry_key(dataset_id, version))
            .cloned()
            .ok_or_else(|| DataStoreError::MissingDataset(registry_key(dataset_id, version)))?;
        verify_artifacts(&self.root, &record)?;
        let metadata: DatasetMetadata = read_json(&self.root.join(&record.metadata_path))?;
        validate_metadata(&metadata)
            .map_err(|err| DataStoreError::InvalidMetadata(format!("{err:?}")))?;
        let bars = read_jsonl_bars(&self.root.join(&record.normalized_path))?;
        Ok(StoredOhlcvDataset {
            record,
            metadata,
            bars,
        })
    }

    fn write_dataset(
        &self,
        mut versioned: VersionedDataset,
        bars: Vec<OhlcvBar>,
        (raw_body, raw_format): (Vec<u8>, OhlcvFormat),
        (raw_request, redacted_headers): (serde_json::Value, serde_json::Value),
        provider_notes: String,
        created_at: String,
    ) -> Result<StoredDatasetRecord, DataStoreError> {
        let dir = self.dataset_dir(&versioned.metadata.dataset_id, &versioned.metadata.version);
        std::fs::create_dir_all(dir.join("raw")).map_err(io_error)?;
        let raw_path = dir.join("raw").join(raw_filename(raw_format));
        let request_path = dir.join("raw/request.json");
        let headers_path = dir.join("raw/headers.redacted.json");
        let notes_path = dir.join("raw/provider-notes.md");
        let metadata_path = dir.join("metadata.json");
        let normalized_path = dir.join("ohlcv.jsonl");
        let validation_path = dir.join("validation.json");
        let manifest_path = dir.join("manifest.json");
        write_bytes(&raw_path, &raw_body)?;
        write_json(&request_path, &raw_request)?;
        write_json(&headers_path, &redacted_headers)?;
        write_text(&notes_path, &provider_notes)?;
        write_jsonl_bars(&normalized_path, &bars)?;
        enrich_metadata_artifacts(
            &self.root,
            &mut versioned.metadata,
            &raw_body,
            (&normalized_path, &raw_path),
            (&validation_path, &manifest_path),
            &created_at,
        )?;
        versioned.content_hash = versioned.metadata.checksum.clone();
        versioned.status = status_from_metadata(&versioned.metadata);
        validate_metadata(&versioned.metadata)
            .map_err(|err| DataStoreError::InvalidMetadata(format!("{err:?}")))?;
        let validation = validation_report(&versioned.metadata, &bars, created_at.clone());
        reconcile_versioned_from_validation(&mut versioned, &validation);
        write_schema_json(&validation_path, &validation)?;
        write_schema_json(&metadata_path, &versioned.metadata)?;
        let record = record(
            &self.root,
            &versioned,
            &bars,
            ArtifactPaths {
                metadata: &metadata_path,
                normalized: &normalized_path,
                raw: &raw_path,
                validation: &validation_path,
                manifest: &manifest_path,
            },
            created_at,
        )?;
        write_schema_json(&manifest_path, &record)?;
        verify_artifacts(&self.root, &record)?;
        let migration = self.load_registry_migration(true)?;
        let mut registry = migration.registry;
        registry.schema_version = REGISTRY_SCHEMA_V2.into();
        let backup = registry_backup_path(&self.data_root(), &record.created_at);
        registry.last_updated = record.created_at.clone();
        registry.datasets.insert(
            registry_key(&record.dataset_id, &record.version),
            record.clone(),
        );
        write_schema_json_with_backup(&self.registry_path(), &registry, &backup)?;
        Ok(record)
    }

    pub fn load_registry(&self) -> Result<PersistentDatasetRegistry, DataStoreError> {
        self.load_verified_registry()
    }

    fn load_verified_registry(&self) -> Result<PersistentDatasetRegistry, DataStoreError> {
        let mut registry = self.load_registry_migration(false)?.registry;
        let mut reconciled = false;
        for record in registry.datasets.values_mut() {
            verify_artifacts(&self.root, record)?;
            let validation =
                read_json::<ValidationReport>(&self.root.join(&record.validation_path));
            let production_eligible = validation
                .as_ref()
                .is_ok_and(ValidationReport::allows_production);
            let status = if production_eligible {
                DatasetStatus::Healthy
            } else {
                DatasetStatus::Degraded
            };
            if record.production_eligible != production_eligible || record.status != status {
                record.production_eligible = production_eligible;
                record.status = status;
                reconciled = true;
            }
        }
        if reconciled {
            write_schema_json(&self.registry_path(), &registry)?;
        }
        Ok(registry)
    }

    fn load_registry_migration(
        &self,
        write_reports: bool,
    ) -> Result<RegistryMigration, DataStoreError> {
        let path = self.registry_path();
        if !path.exists() {
            let registry = PersistentDatasetRegistry::default();
            if write_reports {
                write_schema_json(&path, &registry)?;
            }
            return Ok(RegistryMigration {
                registry,
                report: RegistryMigrationReport {
                    schema_version: REGISTRY_SCHEMA_V2.into(),
                    ..RegistryMigrationReport::default()
                },
            });
        }
        let registry: PersistentDatasetRegistry = read_json(&path)?;
        migrate_registry(&self.root, &self.data_root(), registry, write_reports)
    }

    fn dataset_dir(&self, dataset_id: &str, version: &str) -> PathBuf {
        self.data_root()
            .join("datasets")
            .join(safe_path(dataset_id))
            .join(safe_path(version))
    }
}

#[cfg(test)]
mod data_store_ahdm_tests;
#[cfg(test)]
mod data_store_schema_tests;
#[cfg(test)]
mod data_store_tests;
