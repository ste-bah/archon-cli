use crate::backtest::{BacktestConfig, EvidenceSource};
use crate::candle_backtest::{OhlcvBacktestReport, run_ahdm_shared_manifest_backtest};
use crate::custom_strategy::{
    ComparisonOp, CustomOhlcvStrategy, OhlcvCondition, OhlcvIndicator, OhlcvOperand,
};
use crate::data_lake::{
    BacktestDataGateReport, CoverageCell, CoverageGap, CoverageMatrix, CoverageValidationPolicy,
    DatasetArtifactPaths, DatasetChecksums, DatasetMetadata, DatasetSourceMetadata, DatasetStatus,
    NativeLineageEvidence, ProviderCapabilityResult, SessionCalendarEvidence, ValidationCheck,
    ValidationReport, ValidationSeverity, ValidationStatus, ValidationSummary, VersionedDataset,
    VolumeAbsenceEvidence, can_fetch_symbol_timeframe, dataset_id, raw_bound_version,
    status_from_metadata, validate_metadata,
};
use crate::ohlcv::{
    OhlcvBacktestRequest, OhlcvBacktestRule, OhlcvBar, OhlcvDatasetRef, OhlcvFormat,
    bytes_checksum, coverage_bounds, validate_bars,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

mod ahdm;
mod ahdm_evidence;
mod ahdm_methods;
mod ahdm_readiness;
#[cfg(test)]
mod ahdm_test_support;
mod artifact_schema;
mod backtest_gates;
mod coverage;
mod coverage_methods;
mod gates;
mod io;
mod migration;
mod provider_methods;
mod records;
mod registry;
mod stooq;
mod util;
mod validation;
mod verify_methods;

use ahdm::*;
use ahdm_evidence::*;
use ahdm_readiness::*;
use artifact_schema::*;
use coverage::*;
use gates::*;
use io::*;
use migration::*;
use records::*;
use registry::*;
use stooq::*;
pub use types::*;
use util::*;
use validation::*;

mod types;

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

    pub fn snapshot_dir(&self, provider: &str) -> PathBuf {
        self.data_root().join("snapshots").join(safe_path(provider))
    }

    pub fn snapshot_path(&self, provider: &str, symbol: &str) -> PathBuf {
        self.snapshot_dir(provider)
            .join(format!("{}.json", safe_path(symbol)))
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
        let raw_checksum = bytes_checksum(&request.raw_body);
        let mut metadata = request.metadata;
        if !metadata
            .version
            .as_bytes()
            .get(0..9)
            .is_some_and(|prefix| prefix[..8].iter().all(u8::is_ascii_digit) && prefix[8] == b'-')
        {
            metadata.dataset_id = dataset_id(&metadata).ok_or_else(|| {
                DataStoreError::InvalidMetadata("invalid dataset identity component".into())
            })?;
            metadata.version =
                raw_bound_version(&request.created_at, &raw_checksum).ok_or_else(|| {
                    DataStoreError::InvalidMetadata(
                        "invalid ingestion timestamp or raw checksum".into(),
                    )
                })?;
        }
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
        let serialized_metadata = serde_json::to_value(&metadata)
            .map_err(|error| DataStoreError::Json(error.to_string()))?;
        let native_lineage: Option<NativeLineageEvidence> = request
            .raw_request
            .get("native_lineage_evidence")
            .cloned()
            .and_then(|value| serde_json::from_value(value).ok());
        if contains_secret_material(&request.redacted_headers)
            || contains_secret_material(&request.raw_request)
            || contains_secret_material(&serialized_metadata)
            || contains_secret_bytes(&request.raw_body)
            || contains_secret_text(&request.provider_notes)
        {
            return Err(DataStoreError::InvalidMetadata(
                "secret material rejected".into(),
            ));
        }
        if !native_lineage
            .as_ref()
            .is_some_and(|evidence| native_lineage_matches(&metadata, evidence))
        {
            metadata.production_eligible = false;
            metadata.quality_status = "degraded".into();
        }
        fail_closed_stooq_short_span_metadata(&mut metadata, &request.raw_request);
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
            if existing.checksum == versioned.content_hash && existing.raw_checksum == raw_checksum
            {
                return Ok(existing.clone());
            }
            return Err(DataStoreError::InvalidMetadata(
                "dataset id/version already exists with different normalized or raw bytes".into(),
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
        let metadata = read_dataset_metadata(&self.root, &record)?;
        let bars = read_jsonl_bars(&self.root.join(&record.normalized_path))?;
        let dataset = StoredOhlcvDataset {
            record,
            metadata,
            bars,
        };
        let evidence = load_volume_absence_evidence(&self.root, &dataset.metadata);
        let report = validation_report_at_root_with_volume_evidence(
            &self.root,
            &dataset.metadata,
            &dataset.bars,
            validated_at,
            evidence.as_ref(),
        );
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
        self.evaluate_backtest_data_gate(dataset_id, version, diagnostic_allow_degraded_data)
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
        let metadata = read_dataset_metadata(&self.root, &record)?;
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
        let validation =
            validation_report_at_root(&self.root, &versioned.metadata, &bars, created_at.clone());
        reconcile_versioned_from_validation(&mut versioned, &validation);
        versioned.metadata.checksums.metadata_sha256 = metadata_sha256(&versioned.metadata)?;
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
            &validation,
            created_at,
        )?;
        write_schema_json(&manifest_path, &record)?;
        verify_artifacts(&self.root, &record)?;
        let migration = self.load_registry_migration(true)?;
        let mut registry = migration.registry;
        registry.schema_version = REGISTRY_SCHEMA_V1.into();
        let backup = registry_backup_path(&self.data_root(), &record.created_at);
        registry.last_updated = record.created_at.clone();
        registry.datasets.insert(
            registry_key(&record.dataset_id, &record.version),
            record.clone(),
        );
        write_schema_json_with_backup(&self.registry_path(), &registry, &backup)?;
        Ok(record)
    }

    fn dataset_dir(&self, dataset_id: &str, version: &str) -> PathBuf {
        self.data_root()
            .join("datasets")
            .join(safe_path(dataset_id))
            .join(safe_path(version))
    }
}

#[cfg(test)]
mod artifact_contract_tests;
#[cfg(test)]
mod data_store_ahdm_helpers_tests;
#[cfg(test)]
mod data_store_ahdm_tests;
#[cfg(test)]
mod data_store_schema_tests;
#[cfg(test)]
mod data_store_tests;
#[cfg(test)]
mod validation_tests;
