use super::{DataStoreError, StoreOhlcvRequest, StoredDatasetRecord, TradingDataLake};
use crate::data_lake::providers::stooq::{PreparedStooqHistory, StooqAdapter};
use crate::data_lake::{
    CoverageWindow, DataType, DatasetArtifactPaths, DatasetChecksums, DatasetMetadata,
    DatasetSourceMetadata, DerivationLineage, GapSummary, NativeFetchRequest,
    NativeLineageEvidence, NativeObservationEvidence, UnavailableReason, raw_bound_version,
};
use crate::ohlcv::{OhlcvFormat, bytes_checksum};
use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StooqIngestRequest {
    pub fetch: NativeFetchRequest,
    pub created_at: String,
    pub price_basis: String,
    pub optional: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub enum StooqIngestResult {
    Published(Box<StoredDatasetRecord>),
    Unavailable { reason: UnavailableReason },
}

impl TradingDataLake {
    pub fn ingest_stooq(
        &self,
        adapter: &StooqAdapter,
        request: StooqIngestRequest,
    ) -> Result<StooqIngestResult, DataStoreError> {
        let prepared = match adapter.fetch_for_ingest(&request.fetch) {
            Ok(prepared) => prepared,
            Err(reason) => return Ok(StooqIngestResult::Unavailable { reason }),
        };
        validate_publishable(&request, &prepared)?;
        self.publish_stooq(store_request(request, prepared)?)
    }

    fn publish_stooq(
        &self,
        request: StoreOhlcvRequest,
    ) -> Result<StooqIngestResult, DataStoreError> {
        let publication_dir =
            self.dataset_dir(&request.metadata.dataset_id, &request.metadata.version);
        if publication_dir.exists() {
            return Err(DataStoreError::InvalidMetadata(
                "unregistered Stooq publication directory already exists".into(),
            ));
        }
        match self.store_ohlcv(request) {
            Ok(record) => Ok(StooqIngestResult::Published(Box::new(record))),
            Err(error) => {
                if publication_dir.exists() {
                    std::fs::remove_dir_all(&publication_dir).map_err(super::io_error)?;
                    remove_empty_publication_parents(&publication_dir, &self.data_root())?;
                }
                Err(error)
            }
        }
    }
}

fn remove_empty_publication_parents(
    publication_dir: &std::path::Path,
    data_root: &std::path::Path,
) -> Result<(), DataStoreError> {
    let datasets_root = data_root.join("datasets");
    let mut parent = publication_dir.parent();
    while let Some(path) = parent {
        if path == data_root || !path.starts_with(&datasets_root) {
            break;
        }
        if std::fs::read_dir(path)
            .map_err(super::io_error)?
            .next()
            .is_some()
        {
            break;
        }
        std::fs::remove_dir(path).map_err(super::io_error)?;
        parent = path.parent();
    }
    Ok(())
}

fn validate_publishable(
    request: &StooqIngestRequest,
    prepared: &PreparedStooqHistory,
) -> Result<(), DataStoreError> {
    let evidence = &prepared.evidence;
    if request.created_at != evidence.retrieved_at
        || chrono::DateTime::parse_from_rfc3339(&request.created_at).is_err()
        || request.price_basis.trim().is_empty()
        || evidence.native_interval != request.fetch.timeframe
        || evidence.expected_bars == 0
        || evidence.expected_bars != evidence.observed_bars
        || evidence.observed_bars != prepared.history.bars.len() as u64
        || prepared.history.coverage_start != request.fetch.start
        || prepared.history.coverage_end != request.fetch.end
        || prepared.history.provenance.resampled
    {
        return Err(DataStoreError::InvalidMetadata(
            "incomplete exact-native Stooq evidence".into(),
        ));
    }
    Ok(())
}

fn store_request(
    request: StooqIngestRequest,
    prepared: PreparedStooqHistory,
) -> Result<StoreOhlcvRequest, DataStoreError> {
    let raw_checksum = bytes_checksum(&prepared.raw_body);
    let mut metadata = metadata(&request, prepared.evidence.expected_bars);
    metadata.dataset_id = crate::data_lake::dataset_id(&metadata)
        .ok_or_else(|| DataStoreError::InvalidMetadata("invalid Stooq dataset identity".into()))?;
    metadata.version = raw_bound_version(&request.created_at, &raw_checksum)
        .ok_or_else(|| DataStoreError::InvalidMetadata("invalid ingestion timestamp".into()))?;
    let lineage = lineage(&metadata, &request.created_at);
    let raw_request = serde_json::json!({
        "transport_provenance": prepared.request_provenance,
        "native_lineage_evidence": lineage,
        "credential_required": false,
        "expected_bars": prepared.evidence.expected_bars,
        "request_fingerprint": prepared.evidence.request_fingerprint,
    });
    Ok(StoreOhlcvRequest {
        metadata,
        bars: prepared.history.bars,
        raw_body: prepared.raw_body,
        raw_format: OhlcvFormat::Csv,
        raw_request,
        redacted_headers: prepared.redacted_headers,
        provider_notes: provider_notes(),
        created_at: request.created_at,
    })
}

fn metadata(request: &StooqIngestRequest, expected_bars: u64) -> DatasetMetadata {
    let mut symbol_map = BTreeMap::new();
    symbol_map.insert(
        request.fetch.canonical_instrument.clone(),
        request.fetch.provider_symbol.clone(),
    );
    DatasetMetadata {
        schema_version: "archon-trading-dataset-v1".into(),
        dataset_id: String::new(),
        version: String::new(),
        canonical_instrument: request.fetch.canonical_instrument.clone(),
        asset_class: "equity".into(),
        provider: "stooq".into(),
        provider_symbol: request.fetch.provider_symbol.clone(),
        timeframe: request.fetch.timeframe.clone(),
        native_interval: true,
        production_eligible: true,
        price_basis: request.price_basis.clone(),
        session: "provider_default".into(),
        data_type: DataType::Ohlcv,
        symbol_map,
        timezone: "UTC".into(),
        adjustment: "provider_reported".into(),
        license: "Stooq terms".into(),
        coverage: CoverageWindow {
            start: request.fetch.start.clone(),
            end: request.fetch.end.clone(),
            expected_bars,
            observed_bars: expected_bars,
        },
        gaps: GapSummary {
            missing_bars: 0,
            expected_bars,
        },
        checksum: String::new(),
        checksums: DatasetChecksums::default(),
        paths: DatasetArtifactPaths::default(),
        source: DatasetSourceMetadata {
            license_notes: "Stooq data under Stooq terms".into(),
            url_or_endpoint: "https://stooq.com/q/d/l/".into(),
            retrieved_at: request.created_at.clone(),
            credential_required: false,
        },
        quality_status: "passed".into(),
        created_at: request.created_at.clone(),
        optional: request.optional,
    }
}

fn lineage(metadata: &DatasetMetadata, retrieved_at: &str) -> NativeLineageEvidence {
    NativeLineageEvidence {
        observation: NativeObservationEvidence {
            dataset_id: metadata.dataset_id.clone(),
            version: metadata.version.clone(),
            provider: "stooq".into(),
            canonical_instrument: metadata.canonical_instrument.clone(),
            provider_symbol: metadata.provider_symbol.clone(),
            timeframe: metadata.timeframe.clone(),
            retrieved_at: retrieved_at.into(),
            exact_native_interval: true,
            complete: true,
        },
        lineage: DerivationLineage::default(),
    }
}

fn provider_notes() -> String {
    "Transport: direct Stooq CSV download\nNo aggregation, resampling, sorting, repair, retry, or provider substitution was applied.\n".into()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data_lake::providers::stooq::{
        StooqClock, StooqRequest, StooqResponse, StooqTransport, StooqTransportError,
    };
    use crate::data_store::inject_io_failure;
    use std::sync::Arc;

    struct Clock;

    impl StooqClock for Clock {
        fn now_rfc3339(&self) -> String {
            "2025-01-06T12:00:00Z".into()
        }
    }

    struct Transport;

    impl StooqTransport for Transport {
        fn execute(&self, _request: &StooqRequest) -> Result<StooqResponse, StooqTransportError> {
            Ok(StooqResponse {
                status: 200,
                content_type: "text/csv".into(),
                headers: serde_json::json!({"server": "stooq"}),
                body: b"Date,Open,High,Low,Close,Volume\n2025-01-03,10,12,9,11,100\n2025-01-06,11,13,10,12,200\n".to_vec(),
            })
        }
    }

    fn request() -> StooqIngestRequest {
        StooqIngestRequest {
            fetch: NativeFetchRequest {
                provider: "stooq".into(),
                canonical_instrument: "SPY".into(),
                provider_symbol: "SPY.US".into(),
                timeframe: "1d".into(),
                start: "2025-01-03".into(),
                end: "2025-01-06".into(),
            },
            created_at: "2025-01-06T12:00:00Z".into(),
            price_basis: "raw".into(),
            optional: false,
        }
    }

    #[test]
    fn stooq_publication_failure_removes_dataset_and_registry_state() {
        let temp = tempfile::tempdir().unwrap();
        let lake = TradingDataLake::new(temp.path());
        let adapter = StooqAdapter::new(Arc::new(Transport), Arc::new(Clock));

        inject_io_failure(Some("file.rename"));
        let result = lake.ingest_stooq(&adapter, request());
        inject_io_failure(None);

        assert!(matches!(result, Err(DataStoreError::Io(_))));
        assert!(!lake.registry_path().exists());
        assert!(!lake.data_root().join("datasets").exists());
    }
}
