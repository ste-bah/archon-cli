use crate::spec_registry::{PromotionStatus, StrategySpec};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DataLakeError {
    MissingField(&'static str),
    DegradedDataset,
    MissingMandatoryData(Vec<DataType>),
    UnsupportedInstrumentClass(InstrumentClass),
    FxOptionsNeedSpecAmendment,
    InvalidDatasetId,
    InvalidVersion,
    NonNativeProductionDataset,
    MetadataIncompleteForProduction,
}

#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DatasetRegistry {
    datasets: BTreeMap<String, VersionedDataset>,
}

impl DatasetRegistry {
    pub fn register(
        &mut self,
        metadata: DatasetMetadata,
    ) -> Result<VersionedDataset, DataLakeError> {
        validate_metadata(&metadata)?;
        let status = status_from_metadata(&metadata);
        let content_hash = dataset_hash(&metadata);
        let versioned = VersionedDataset {
            metadata,
            content_hash,
            status,
        };
        self.datasets
            .insert(registry_key(&versioned.metadata), versioned.clone());
        Ok(versioned)
    }

    pub fn get(&self, dataset_id: &str) -> Option<&VersionedDataset> {
        self.datasets.get(dataset_id).or_else(|| {
            self.datasets
                .values()
                .find(|dataset| dataset.metadata.dataset_id == dataset_id)
        })
    }

    pub fn all(&self) -> impl Iterator<Item = &VersionedDataset> {
        self.datasets.values()
    }

    pub fn promotion_ready(
        &self,
        in_scope: &[InstrumentClass],
        event_driven: bool,
    ) -> Result<(), DataLakeError> {
        let mut present = BTreeSet::new();
        for dataset in self.datasets.values() {
            if dataset.status == DatasetStatus::Degraded {
                if dataset.metadata.optional {
                    continue;
                }
                return Err(DataLakeError::DegradedDataset);
            }
            present.insert(dataset.metadata.data_type);
        }
        let missing = missing_mandatory_data(in_scope, event_driven, &present)?;
        if missing.is_empty() {
            Ok(())
        } else {
            Err(DataLakeError::MissingMandatoryData(missing))
        }
    }
}

pub fn validate_metadata(metadata: &DatasetMetadata) -> Result<(), DataLakeError> {
    require_text(&metadata.dataset_id, "dataset_id")?;
    require_text(&metadata.version, "version")?;
    require_text(&metadata.provider, "provider")?;
    require_text(&metadata.canonical_instrument, "canonical_instrument")?;
    require_text(&metadata.asset_class, "asset_class")?;
    require_text(&metadata.provider_symbol, "provider_symbol")?;
    require_text(&metadata.timeframe, "timeframe")?;
    require_text(&metadata.price_basis, "price_basis")?;
    require_text(&metadata.session, "session")?;
    require_text(&metadata.quality_status, "quality_status")?;
    require_text(&metadata.timezone, "timezone")?;
    require_text(&metadata.adjustment, "adjustment")?;
    require_text(&metadata.license, "license")?;
    require_text(&metadata.coverage.start, "coverage.start")?;
    require_text(&metadata.coverage.end, "coverage.end")?;
    require_text(&metadata.checksum, "checksum")?;
    if metadata.production_eligible {
        require_text(&metadata.checksums.raw_sha256, "checksums.raw_sha256")?;
        require_text(
            &metadata.checksums.normalized_sha256,
            "checksums.normalized_sha256",
        )?;
        require_text(
            &metadata.checksums.metadata_sha256,
            "checksums.metadata_sha256",
        )?;
        require_text(&metadata.paths.raw, "paths.raw")?;
        require_text(&metadata.paths.raw_response, "paths.raw_response")?;
        require_text(&metadata.paths.raw_request, "paths.raw_request")?;
        require_text(&metadata.paths.redacted_headers, "paths.redacted_headers")?;
        require_text(&metadata.paths.provider_notes, "paths.provider_notes")?;
        require_text(&metadata.paths.normalized, "paths.normalized")?;
        require_text(&metadata.paths.validation, "paths.validation")?;
        require_text(&metadata.paths.manifest, "paths.manifest")?;
    }
    if !valid_dataset_id(&metadata.dataset_id) {
        return Err(DataLakeError::InvalidDatasetId);
    }
    if !valid_version(&metadata.version) {
        return Err(DataLakeError::InvalidVersion);
    }
    if !provider_identity_matches_dataset(&metadata.provider, &metadata.dataset_id) {
        return Err(DataLakeError::MetadataIncompleteForProduction);
    }
    if !dataset_id_matches_metadata(metadata) {
        return Err(DataLakeError::MissingField("symbol_map"));
    }
    if metadata.coverage.expected_bars == 0 {
        return Err(DataLakeError::MissingField("coverage.expected_bars"));
    }
    if metadata.gaps.expected_bars == 0 {
        return Err(DataLakeError::MissingField("gaps.expected_bars"));
    }
    if metadata.production_eligible && !metadata.native_interval {
        return Err(DataLakeError::NonNativeProductionDataset);
    }
    if metadata.production_eligible && metadata.quality_status != "passed" {
        return Err(DataLakeError::MetadataIncompleteForProduction);
    }
    if metadata.provider.eq_ignore_ascii_case("yfinance")
        && (metadata.production_eligible || metadata.quality_status != "degraded")
    {
        return Err(DataLakeError::MetadataIncompleteForProduction);
    }
    Ok(())
}

mod contracts;
mod identity;
mod model;
pub use contracts::*;
use identity::{dataset_id_matches_metadata, valid_dataset_id, valid_version};
pub use model::*;

pub fn status_from_gaps(gaps: &GapSummary) -> DatasetStatus {
    if gaps.gap_percent() > 0.01 {
        DatasetStatus::Degraded
    } else {
        DatasetStatus::Healthy
    }
}

pub fn status_from_metadata(metadata: &DatasetMetadata) -> DatasetStatus {
    if !metadata.production_eligible || metadata.quality_status != "passed" {
        DatasetStatus::Degraded
    } else {
        status_from_gaps(&metadata.gaps)
    }
}

pub fn mandatory_data_types(class: InstrumentClass) -> Result<Vec<DataType>, DataLakeError> {
    let values = match class {
        InstrumentClass::Equity => vec![
            DataType::Ohlcv,
            DataType::CorporateActions,
            DataType::Fundamentals,
            DataType::IndexConstituents,
        ],
        InstrumentClass::Crypto => vec![DataType::Ohlcv, DataType::Funding],
        InstrumentClass::Future => vec![
            DataType::Ohlcv,
            DataType::ContinuousContract,
            DataType::ContractSpecs,
        ],
        InstrumentClass::Fx | InstrumentClass::Option => {
            return Err(DataLakeError::UnsupportedInstrumentClass(class));
        }
    };
    Ok(values)
}

pub fn spec_can_advance_past_idea(spec: &StrategySpec) -> Result<(), DataLakeError> {
    let Some(status) = spec.spec_f15_promotion_status else {
        return Err(DataLakeError::FxOptionsNeedSpecAmendment);
    };
    let has_unsupported = spec
        .spec_f01_instrument_universe
        .as_ref()
        .is_some_and(|items| items.iter().any(|item| is_fx_or_option(&item.asset_class)));
    if has_unsupported && status > PromotionStatus::Idea {
        Err(DataLakeError::FxOptionsNeedSpecAmendment)
    } else {
        Ok(())
    }
}

fn missing_mandatory_data(
    in_scope: &[InstrumentClass],
    event_driven: bool,
    present: &BTreeSet<DataType>,
) -> Result<Vec<DataType>, DataLakeError> {
    let mut required = BTreeSet::new();
    for class in in_scope {
        for data_type in mandatory_data_types(*class)? {
            required.insert(data_type);
        }
    }
    if event_driven {
        required.insert(DataType::News);
    }
    Ok(required.difference(present).copied().collect())
}

fn dataset_hash(metadata: &DatasetMetadata) -> String {
    let bytes = serde_json::to_vec(metadata).unwrap_or_default();
    blake3::hash(&bytes).to_hex().to_string()
}

fn registry_key(metadata: &DatasetMetadata) -> String {
    format!("{}::{:?}", metadata.dataset_id, metadata.data_type)
}

pub(crate) fn normalize_timeframe(value: &str) -> String {
    match value.trim() {
        "4H" | "4h" => "240".into(),
        "1H" | "1h" => "60".into(),
        "15m" | "15M" => "15".into(),
        other => other.into(),
    }
}

pub(crate) fn provider_supports_native_timeframe(provider: &str, timeframe: &str) -> bool {
    match provider {
        "stooq" => timeframe == "1D",
        "tradingview" | "openbb" | "polygon" | "yfinance" => true,
        _ => false,
    }
}

pub(super) fn unavailable_reason(
    provider: &str,
    symbol: &str,
    timeframe: &str,
    supported_provider: bool,
    native_interval: bool,
) -> String {
    let provider = provider.trim().to_ascii_lowercase();
    if provider.is_empty() || symbol.trim().is_empty() || timeframe.is_empty() {
        "missing provider, symbol, or timeframe".into()
    } else if let Ok(code) = symbol.trim().parse::<u16>() {
        status_from_http_code(code)
            .unwrap_or("provider returned an unavailable status")
            .into()
    } else if !supported_provider {
        "provider is not registered in the capability interface".into()
    } else if !native_interval && provider == "stooq" {
        "provider_blocked_or_unavailable: exact native Stooq interval unavailable; resampling is refused".into()
    } else if !native_interval {
        "exact native interval is unsupported".into()
    } else if provider == "yfinance" {
        "yfinance fallback is degraded and ineligible for promotion".into()
    } else if matches!(provider.as_str(), "openbb" | "polygon") {
        "missing provider credentials".into()
    } else {
        "provider-specific native fetch support is not implemented in this task".into()
    }
}

fn status_from_http_code(code: u16) -> Option<&'static str> {
    match code {
        401 => Some("missing or invalid provider credentials"),
        403 => Some("provider blocked access"),
        404 => Some("provider symbol or endpoint not found"),
        _ => None,
    }
}

fn provider_identity_matches_dataset(provider: &str, dataset_id: &str) -> bool {
    let provider = provider.trim().to_ascii_lowercase();
    let dataset_id = dataset_id.trim().to_ascii_lowercase();
    let Some((prefix, _)) = dataset_id.split_once('-') else {
        return true;
    };
    match provider.as_str() {
        "polygon" => prefix == "polygon",
        "openbb" => prefix == "openbb",
        "tradingview" => prefix == "tradingview",
        "stooq" => prefix == "stooq",
        "yfinance" => prefix == "yfinance",
        _ => true,
    }
}

fn require_text(value: &str, field: &'static str) -> Result<(), DataLakeError> {
    if value.trim().is_empty() {
        Err(DataLakeError::MissingField(field))
    } else {
        Ok(())
    }
}

fn is_fx_or_option(asset_class: &str) -> bool {
    matches!(
        asset_class.to_ascii_lowercase().as_str(),
        "fx" | "forex" | "option" | "options"
    )
}

#[cfg(test)]
mod tests;
