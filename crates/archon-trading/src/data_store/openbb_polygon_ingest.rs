use super::{DataStoreError, StoreOhlcvRequest, StoredDatasetRecord, TradingDataLake};
use crate::data_lake::providers::openbb_polygon::{
    OpenbbPolygonAdapter, OpenbbPolygonEvidence, PreparedOpenbbPolygonHistory,
};
use crate::data_lake::{
    CoverageWindow, DataType, DatasetArtifactPaths, DatasetChecksums, DatasetMetadata,
    DatasetSourceMetadata, DerivationLineage, GapSummary, NativeFetchRequest,
    NativeLineageEvidence, NativeObservationEvidence, UnavailableReason, raw_bound_version,
};
use crate::ohlcv::{OhlcvFormat, bytes_checksum, validate_bars};
use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpenbbPolygonIngestRequest {
    pub fetch: NativeFetchRequest,
    pub created_at: String,
    pub price_basis: String,
    pub optional: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub enum OpenbbPolygonIngestResult {
    Published(Box<StoredDatasetRecord>),
    Partial {
        observed_bars: u64,
        expected_bars: u64,
    },
    Unavailable {
        reason: UnavailableReason,
    },
}

impl TradingDataLake {
    pub fn ingest_openbb_polygon(
        &self,
        adapter: &OpenbbPolygonAdapter,
        request: OpenbbPolygonIngestRequest,
    ) -> Result<OpenbbPolygonIngestResult, DataStoreError> {
        let prepared = match adapter.fetch_for_ingest(&request.fetch) {
            Ok(prepared) => prepared,
            Err(reason) => return Ok(OpenbbPolygonIngestResult::Unavailable { reason }),
        };
        if !complete_history(&request.fetch, &prepared) {
            return Ok(OpenbbPolygonIngestResult::Partial {
                observed_bars: prepared.history.bars.len() as u64,
                expected_bars: prepared.evidence.expected_bars,
            });
        }
        validate_publishable(&request, &prepared)?;
        let store_request = store_request(request, prepared)?;
        self.publish_openbb_polygon(store_request)
    }

    fn publish_openbb_polygon(
        &self,
        request: StoreOhlcvRequest,
    ) -> Result<OpenbbPolygonIngestResult, DataStoreError> {
        let publication_dir =
            self.dataset_dir(&request.metadata.dataset_id, &request.metadata.version);
        if publication_dir.exists() {
            return Err(DataStoreError::InvalidMetadata(
                "unregistered OpenBB/Polygon publication directory already exists".into(),
            ));
        }
        match self.store_ohlcv(request) {
            Ok(record) => Ok(OpenbbPolygonIngestResult::Published(Box::new(record))),
            Err(error) => {
                if publication_dir.exists() {
                    std::fs::remove_dir_all(&publication_dir).map_err(super::io_error)?;
                }
                Err(error)
            }
        }
    }
}

fn complete_history(request: &NativeFetchRequest, prepared: &PreparedOpenbbPolygonHistory) -> bool {
    let history = &prepared.history;
    history.coverage_start == request.start
        && history.coverage_end == request.end
        && prepared.evidence.expected_bars == history.bars.len() as u64
}

fn validate_publishable(
    request: &OpenbbPolygonIngestRequest,
    prepared: &PreparedOpenbbPolygonHistory,
) -> Result<(), DataStoreError> {
    if request.created_at != prepared.evidence.retrieved_at
        || chrono::DateTime::parse_from_rfc3339(&request.created_at).is_err()
        || request.price_basis.trim().is_empty()
        || prepared.evidence.transport != "openbb"
        || prepared.evidence.corporate_action_source != "polygon"
        || prepared.evidence.asset_class != "equity"
        || !has_complete_evidence(&prepared.evidence)
    {
        return Err(DataStoreError::InvalidMetadata(
            "incomplete OpenBB/Polygon provenance".into(),
        ));
    }
    validate_bars(&prepared.history.bars)
        .map_err(|error| DataStoreError::InvalidOhlcv(format!("{error:?}")))?;
    if !non_degenerate_volume(&prepared.history.bars) {
        return Err(DataStoreError::InvalidOhlcv(
            "production history requires non-degenerate volume".into(),
        ));
    }
    Ok(())
}

fn has_complete_evidence(evidence: &OpenbbPolygonEvidence) -> bool {
    [
        &evidence.route,
        &evidence.adjustment,
        &evidence.corporate_action_source,
        &evidence.exchange,
        &evidence.timezone,
        &evidence.session,
        &evidence.calendar,
        &evidence.license,
        &evidence.response_schema,
        &evidence.response_version,
    ]
    .iter()
    .all(|value| !value.trim().is_empty())
}

fn non_degenerate_volume(bars: &[crate::ohlcv::OhlcvBar]) -> bool {
    let Some(first) = bars.first().map(|bar| bar.volume) else {
        return false;
    };
    first > 0.0
        && bars
            .iter()
            .skip(1)
            .any(|bar| bar.volume > 0.0 && bar.volume != first)
}

fn store_request(
    request: OpenbbPolygonIngestRequest,
    prepared: PreparedOpenbbPolygonHistory,
) -> Result<StoreOhlcvRequest, DataStoreError> {
    let raw_body = serde_json::to_vec(&prepared.raw_response)
        .map_err(|error| DataStoreError::Json(error.to_string()))?;
    let raw_checksum = bytes_checksum(&raw_body);
    let mut metadata = metadata(&request, &prepared.evidence);
    metadata.dataset_id = crate::data_lake::dataset_id(&metadata).ok_or_else(|| {
        DataStoreError::InvalidMetadata("invalid Polygon dataset identity".into())
    })?;
    metadata.version = raw_bound_version(&request.created_at, &raw_checksum)
        .ok_or_else(|| DataStoreError::InvalidMetadata("invalid ingestion timestamp".into()))?;
    let lineage = lineage(&metadata, &request.created_at);
    let raw_request = request_artifact(prepared.request_provenance, lineage);
    let notes = provider_notes(&prepared.evidence);
    Ok(StoreOhlcvRequest {
        metadata,
        bars: prepared.history.bars,
        raw_body,
        raw_format: OhlcvFormat::Json,
        raw_request,
        redacted_headers: prepared.redacted_headers,
        provider_notes: notes,
        created_at: request.created_at,
    })
}

fn metadata(
    request: &OpenbbPolygonIngestRequest,
    evidence: &OpenbbPolygonEvidence,
) -> DatasetMetadata {
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
        asset_class: evidence.asset_class.clone(),
        provider: "polygon".into(),
        provider_symbol: request.fetch.provider_symbol.clone(),
        timeframe: request.fetch.timeframe.clone(),
        native_interval: true,
        production_eligible: true,
        price_basis: request.price_basis.clone(),
        session: evidence.session.clone(),
        data_type: DataType::Ohlcv,
        symbol_map,
        timezone: evidence.timezone.clone(),
        adjustment: evidence.adjustment.clone(),
        license: evidence.license.clone(),
        coverage: CoverageWindow {
            start: request.fetch.start.clone(),
            end: request.fetch.end.clone(),
            expected_bars: evidence.expected_bars,
            observed_bars: evidence.expected_bars,
        },
        gaps: GapSummary {
            missing_bars: 0,
            expected_bars: evidence.expected_bars,
        },
        checksum: String::new(),
        checksums: DatasetChecksums::default(),
        paths: DatasetArtifactPaths::default(),
        source: DatasetSourceMetadata {
            license_notes: format!("Polygon data under {}", evidence.license),
            url_or_endpoint: format!("openbb:{}", evidence.route),
            retrieved_at: request.created_at.clone(),
            credential_required: true,
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
            provider: "polygon".into(),
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

fn request_artifact(
    provenance: serde_json::Value,
    lineage: NativeLineageEvidence,
) -> serde_json::Value {
    serde_json::json!({
        "transport_provenance": provenance,
        "native_lineage_evidence": lineage,
        "credential_required": true,
    })
}

fn provider_notes(evidence: &OpenbbPolygonEvidence) -> String {
    format!(
        "Transport: OpenBB\nProvider: Polygon\nRoute: {}\nExchange: {}\nCalendar: {}\nCorporate actions: {} ({})\nResponse schema: {} {}\nNo aggregation, resampling, sorting, or repair was applied.\n",
        evidence.route,
        evidence.exchange,
        evidence.calendar,
        evidence.adjustment,
        evidence.corporate_action_source,
        evidence.response_schema,
        evidence.response_version,
    )
}

#[cfg(test)]
#[path = "openbb_polygon_ingest_tests.rs"]
mod openbb_polygon_ingest_tests;
