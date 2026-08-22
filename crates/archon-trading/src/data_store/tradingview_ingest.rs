use super::*;
use crate::data_lake::{
    CoverageWindow, DataType, DatasetArtifactPaths, DatasetChecksums, DatasetMetadata,
    DatasetSourceMetadata, DerivationLineage, GapSummary, NativeLineageEvidence,
    NativeObservationEvidence, TradingViewNativeHistory,
};

const PROVIDER: &str = "tradingview";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TradingViewPublication {
    pub asset_class: String,
    pub session: String,
    pub timezone: String,
    pub adjustment: String,
    pub license: String,
    pub created_at: String,
}

impl TradingDataLake {
    pub fn publish_tradingview_history(
        &self,
        history: TradingViewNativeHistory,
        publication: TradingViewPublication,
    ) -> Result<StoredDatasetRecord, DataStoreError> {
        validate_publication_input(&history, &publication)?;
        let raw_body = serde_json::to_vec(&history.raw_response)
            .map_err(|error| DataStoreError::Json(error.to_string()))?;
        let raw_checksum = bytes_checksum(&raw_body);
        let dataset_id = format!(
            "{PROVIDER}-{}-{}-raw",
            history.request.canonical_instrument, history.request.timeframe
        );
        let version =
            raw_bound_version(&publication.created_at, &raw_checksum).ok_or_else(|| {
                DataStoreError::InvalidMetadata("invalid TradingView publication timestamp".into())
            })?;
        let raw_request =
            history_request_artifact(&history, &dataset_id, &version, &publication.created_at);
        let request = StoreOhlcvRequest {
            metadata: metadata(&history, &publication, dataset_id, version),
            bars: history.bars,
            raw_body,
            raw_format: OhlcvFormat::Json,
            raw_request,
            redacted_headers: serde_json::json!({}),
            provider_notes: format!(
                "TradingView chart-equivalent research data fetched through mandatory MCP sequence: {}/{}/{}. requested_bars={}, returned_bars={}. Not institutional vendor data.",
                crate::data_lake::TV_HEALTH_CHECK,
                crate::data_lake::TV_CHART_GET_STATE,
                crate::data_lake::TV_DATA_GET_OHLCV,
                history.request.expected_bars,
                history.returned_bars,
            ),
            created_at: publication.created_at,
        };
        let dataset_dir = self.dataset_dir(&request.metadata.dataset_id, &request.metadata.version);
        match self.store_ohlcv(request) {
            Ok(record) => Ok(record),
            Err(error) => {
                let _ = std::fs::remove_dir_all(dataset_dir);
                Err(error)
            }
        }
    }

    pub fn publish_tradingview_snapshot(
        &self,
        snapshot: crate::data_lake::CurrentSnapshot,
        now_unix_seconds: i64,
    ) -> Result<PathBuf, DataStoreError> {
        let expected_symbol = crate::data_lake::tradingview_symbol(&snapshot.canonical_instrument);
        if snapshot.provider != PROVIDER
            || expected_symbol != Some(snapshot.provider_symbol.as_str())
            || contains_secret_material(&snapshot.payload)
        {
            return Err(DataStoreError::InvalidMetadata(
                "foreign-identity or secret-bearing TradingView snapshot rejected".into(),
            ));
        }
        self.persist_snapshot(snapshot, now_unix_seconds)
    }
}

fn validate_publication_input(
    history: &TradingViewNativeHistory,
    publication: &TradingViewPublication,
) -> Result<(), DataStoreError> {
    if history.returned_bars != history.request.expected_bars
        || history.returned_bars != history.bars.len() as u64
        || history.action_counts
            != crate::data_lake::TradingViewMcpActionCounts::exact_history_sequence()
        || contains_secret_material(&history.raw_response)
    {
        return Err(DataStoreError::InvalidOhlcv(
            "limited or unsafe TradingView history cannot be published".into(),
        ));
    }
    let expected_symbol =
        crate::data_lake::tradingview_symbol(&history.request.canonical_instrument);
    if expected_symbol != Some(history.request.provider_symbol.as_str())
        || !crate::data_lake::TRADINGVIEW_NATIVE_TIMEFRAMES
            .contains(&history.request.timeframe.as_str())
    {
        return Err(DataStoreError::InvalidMetadata(
            "non-native TradingView identity rejected".into(),
        ));
    }
    if publication.asset_class.trim().is_empty()
        || publication.session.trim().is_empty()
        || publication.timezone.trim().is_empty()
        || publication.adjustment.trim().is_empty()
        || publication.license.trim().is_empty()
    {
        return Err(DataStoreError::InvalidMetadata(
            "TradingView publication metadata is incomplete".into(),
        ));
    }
    Ok(())
}

fn metadata(
    history: &TradingViewNativeHistory,
    publication: &TradingViewPublication,
    dataset_id: String,
    version: String,
) -> DatasetMetadata {
    let expected_bars = history.request.expected_bars;
    DatasetMetadata {
        schema_version: "archon-trading-dataset-v1".into(),
        dataset_id,
        version,
        canonical_instrument: history.request.canonical_instrument.clone(),
        asset_class: publication.asset_class.clone(),
        provider: PROVIDER.into(),
        provider_symbol: history.request.provider_symbol.clone(),
        timeframe: history.request.timeframe.clone(),
        native_interval: true,
        production_eligible: true,
        price_basis: "raw".into(),
        session: publication.session.clone(),
        data_type: DataType::Ohlcv,
        symbol_map: BTreeMap::from([(
            history.request.canonical_instrument.clone(),
            history.request.provider_symbol.clone(),
        )]),
        timezone: publication.timezone.clone(),
        adjustment: publication.adjustment.clone(),
        license: publication.license.clone(),
        coverage: CoverageWindow {
            start: history.request.start.clone(),
            end: history.request.end.clone(),
            expected_bars,
            observed_bars: history.returned_bars,
        },
        gaps: GapSummary {
            missing_bars: 0,
            expected_bars,
        },
        checksum: String::new(),
        checksums: DatasetChecksums::default(),
        paths: DatasetArtifactPaths::default(),
        source: DatasetSourceMetadata {
            license_notes: "chart-equivalent research data; not institutional vendor data".into(),
            url_or_endpoint: crate::data_lake::TV_DATA_GET_OHLCV.into(),
            retrieved_at: publication.created_at.clone(),
            credential_required: false,
        },
        quality_status: "passed".into(),
        created_at: publication.created_at.clone(),
        optional: false,
    }
}

fn history_request_artifact(
    history: &TradingViewNativeHistory,
    dataset_id: &str,
    version: &str,
    retrieved_at: &str,
) -> serde_json::Value {
    let request = &history.request;
    let native_lineage_evidence = NativeLineageEvidence {
        observation: NativeObservationEvidence {
            dataset_id: dataset_id.into(),
            version: version.into(),
            provider: PROVIDER.into(),
            canonical_instrument: request.canonical_instrument.clone(),
            provider_symbol: request.provider_symbol.clone(),
            timeframe: request.timeframe.clone(),
            retrieved_at: retrieved_at.into(),
            exact_native_interval: true,
            complete: true,
        },
        lineage: DerivationLineage::default(),
    };
    let fingerprint = blake3::hash(serde_json::to_vec(request).unwrap_or_default().as_slice())
        .to_hex()
        .to_string();
    serde_json::json!({
        "provider": PROVIDER,
        "sequence": [
            crate::data_lake::TV_HEALTH_CHECK,
            crate::data_lake::TV_CHART_GET_STATE,
            crate::data_lake::TV_DATA_GET_OHLCV,
        ],
        "action_counts": {
            crate::data_lake::TV_HEALTH_CHECK: history.action_counts.health_check,
            crate::data_lake::TV_CHART_GET_STATE: history.action_counts.chart_get_state,
            crate::data_lake::TV_DATA_GET_OHLCV: history.action_counts.data_get_ohlcv,
            crate::data_lake::TV_QUOTE_GET: history.action_counts.quote_get,
        },
        "redacted_request_fingerprint": fingerprint,
        "request": request,
        "returned_bars": history.returned_bars,
        "native_lineage_evidence": native_lineage_evidence,
    })
}

#[cfg(test)]
#[path = "tradingview_ingest_tests.rs"]
mod tests;
