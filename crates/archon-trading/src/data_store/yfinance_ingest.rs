use super::{DataStoreError, StoreOhlcvRequest, StoredDatasetRecord, TradingDataLake};
use crate::data_lake::providers::yfinance::{PreparedYfinanceHistory, YfinanceAdapter};
use crate::data_lake::{
    CoverageWindow, DataType, DatasetArtifactPaths, DatasetChecksums, DatasetMetadata,
    DatasetSourceMetadata, GapSummary, NativeFetchRequest, UnavailableReason, raw_bound_version,
};
use crate::ohlcv::{OhlcvFormat, bytes_checksum};
use std::collections::BTreeMap;

const YFINANCE_ADAPTER_RUNTIME_VERSION: &str = "native-yahoo-chart-v1";
const YFINANCE_RETENTION_LIMITATION: &str =
    "Yahoo intraday retention is interval-dependent and not guaranteed";
const YFINANCE_SOURCE_LIMITATION: &str = "unofficial degraded diagnostic fallback only";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct YfinanceIngestRequest {
    pub fetch: NativeFetchRequest,
    pub created_at: String,
    pub price_basis: String,
    pub optional: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub enum YfinanceIngestResult {
    PublishedDegradedDiagnostic(Box<StoredDatasetRecord>),
    Partial { observed_bars: u64 },
    Unavailable { reason: UnavailableReason },
}

impl TradingDataLake {
    pub fn ingest_yfinance(
        &self,
        adapter: &YfinanceAdapter,
        request: YfinanceIngestRequest,
    ) -> Result<YfinanceIngestResult, DataStoreError> {
        let prepared = match adapter.fetch_for_ingest(&request.fetch) {
            Ok(prepared) => prepared,
            Err(reason) => return Ok(YfinanceIngestResult::Unavailable { reason }),
        };
        if !is_complete(&request.fetch, &prepared) {
            return Ok(YfinanceIngestResult::Partial {
                observed_bars: prepared.history.bars.len() as u64,
            });
        }
        validate_publishable(&request, &prepared)?;
        self.publish_yfinance(store_request(request, prepared)?)
    }

    fn publish_yfinance(
        &self,
        request: StoreOhlcvRequest,
    ) -> Result<YfinanceIngestResult, DataStoreError> {
        let publication_dir =
            self.dataset_dir(&request.metadata.dataset_id, &request.metadata.version);
        if publication_dir.exists() {
            return Err(DataStoreError::InvalidMetadata(
                "unregistered yfinance publication directory already exists".into(),
            ));
        }
        match self.store_ohlcv(request) {
            Ok(record) => Ok(YfinanceIngestResult::PublishedDegradedDiagnostic(Box::new(
                record,
            ))),
            Err(error) => {
                if publication_dir.exists() {
                    std::fs::remove_dir_all(&publication_dir).map_err(super::io_error)?;
                }
                Err(error)
            }
        }
    }
}

fn is_complete(request: &NativeFetchRequest, prepared: &PreparedYfinanceHistory) -> bool {
    let history = &prepared.history;
    !history.bars.is_empty()
        && history.coverage_start == request.start
        && history.coverage_end == request.end
        && history.requested_start == request.start
        && history.requested_end == request.end
}

fn validate_publishable(
    request: &YfinanceIngestRequest,
    prepared: &PreparedYfinanceHistory,
) -> Result<(), DataStoreError> {
    let history = &prepared.history;
    if request.created_at != prepared.retrieved_at
        || chrono::DateTime::parse_from_rfc3339(&request.created_at).is_err()
        || request.price_basis.trim().is_empty()
        || request.fetch.provider != "yfinance"
        || history.provider != "yfinance"
        || history.provenance.provider != "yfinance"
        || history.provenance.provider_symbol != request.fetch.provider_symbol
        || history.provenance.native_timeframe != request.fetch.timeframe
        || history.provenance.resampled
    {
        return Err(DataStoreError::InvalidMetadata(
            "incomplete direct-native yfinance evidence".into(),
        ));
    }
    Ok(())
}

fn store_request(
    request: YfinanceIngestRequest,
    prepared: PreparedYfinanceHistory,
) -> Result<StoreOhlcvRequest, DataStoreError> {
    let raw_checksum = bytes_checksum(&prepared.raw_body);
    let expected_bars = prepared.history.bars.len() as u64;
    let mut metadata = metadata(&request, expected_bars);
    metadata.dataset_id = crate::data_lake::dataset_id(&metadata).ok_or_else(|| {
        DataStoreError::InvalidMetadata("invalid yfinance dataset identity".into())
    })?;
    metadata.version = raw_bound_version(&request.created_at, &raw_checksum)
        .ok_or_else(|| DataStoreError::InvalidMetadata("invalid ingestion timestamp".into()))?;
    let raw_request = serde_json::json!({
        "transport_provenance": prepared.request_provenance,
        "diagnostic_only": true,
        "production_eligible": false,
        "promotion_eligible": false,
        "direct_native": true,
        "derived": false,
        "aggregated": false,
        "resampled": false,
        "fallback_contract": {
            "adapter_runtime_version": YFINANCE_ADAPTER_RUNTIME_VERSION,
            "retention": YFINANCE_RETENTION_LIMITATION,
            "adjustment": request.price_basis,
            "session": "provider_default",
            "timezone": "UTC",
            "source_limitations": YFINANCE_SOURCE_LIMITATION,
        },
    });
    Ok(StoreOhlcvRequest {
        metadata,
        bars: prepared.history.bars,
        raw_body: prepared.raw_body,
        raw_format: OhlcvFormat::Json,
        raw_request,
        redacted_headers: prepared.redacted_headers,
        provider_notes: provider_notes(),
        created_at: request.created_at,
    })
}

fn metadata(request: &YfinanceIngestRequest, expected_bars: u64) -> DatasetMetadata {
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
        provider: "yfinance".into(),
        provider_symbol: request.fetch.provider_symbol.clone(),
        timeframe: request.fetch.timeframe.clone(),
        native_interval: true,
        production_eligible: false,
        price_basis: request.price_basis.clone(),
        session: "provider_default".into(),
        data_type: DataType::Ohlcv,
        symbol_map,
        timezone: "UTC".into(),
        adjustment: "provider_reported".into(),
        license: "Yahoo terms".into(),
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
            license_notes: "Unofficial yfinance diagnostic fallback under Yahoo terms".into(),
            url_or_endpoint: "https://query1.finance.yahoo.com/v8/finance/chart".into(),
            retrieved_at: request.created_at.clone(),
            credential_required: false,
        },
        quality_status: "degraded".into(),
        created_at: request.created_at.clone(),
        optional: request.optional,
    }
}

fn provider_notes() -> String {
    format!(
        "Provider: yfinance\nUpstream identity: Yahoo Finance chart API\nAdapter runtime version: {YFINANCE_ADAPTER_RUNTIME_VERSION}\nRetention: {YFINANCE_RETENTION_LIMITATION}\nAdjustment: provider_reported\nSession: provider_default\nTimezone: UTC\nSource limitations: {YFINANCE_SOURCE_LIMITATION}\nPurpose: degraded diagnostic fallback only\nProduction: permanently denied\nPromotion: permanently denied\nNo relabeling, derivation, aggregation, resampling, sorting, repair, retry, or provider substitution was applied.\n"
    )
}

#[cfg(test)]
#[path = "yfinance_ingest_tests.rs"]
mod yfinance_ingest_tests;
