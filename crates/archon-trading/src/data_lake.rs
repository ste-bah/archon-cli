use crate::spec_registry::{PromotionStatus, StrategySpec};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum InstrumentClass {
    Equity,
    Crypto,
    Future,
    Fx,
    Option,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum DataType {
    Ohlcv,
    CorporateActions,
    Fundamentals,
    Borrow,
    Funding,
    IndexConstituents,
    ContinuousContract,
    ContractSpecs,
    News,
    Tick,
    OrderBook,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DatasetStatus {
    Healthy,
    Degraded,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CoverageWindow {
    pub start: String,
    pub end: String,
    pub expected_bars: u64,
    pub observed_bars: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GapSummary {
    pub missing_bars: u64,
    pub expected_bars: u64,
}

impl GapSummary {
    pub fn gap_percent(&self) -> f64 {
        if self.expected_bars == 0 {
            0.0
        } else {
            self.missing_bars as f64 / self.expected_bars as f64
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DatasetMetadata {
    pub dataset_id: String,
    pub provider: String,
    pub data_type: DataType,
    pub symbol_map: BTreeMap<String, String>,
    pub timezone: String,
    pub adjustment: String,
    pub license: String,
    pub coverage: CoverageWindow,
    pub gaps: GapSummary,
    pub checksum: String,
    pub version: String,
    pub optional: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VersionedDataset {
    pub metadata: DatasetMetadata,
    pub content_hash: String,
    pub status: DatasetStatus,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DataLakeError {
    MissingField(&'static str),
    DegradedDataset,
    MissingMandatoryData(Vec<DataType>),
    UnsupportedInstrumentClass(InstrumentClass),
    FxOptionsNeedSpecAmendment,
}

#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DatasetRegistry {
    datasets: BTreeMap<String, VersionedDataset>,
}

impl DatasetRegistry {
    pub fn register(
        &mut self,
        metadata: DatasetMetadata,
    ) -> Result<VersionedDataset, DataLakeError> {
        validate_metadata(&metadata)?;
        let status = status_from_gaps(&metadata.gaps);
        let content_hash = dataset_hash(&metadata);
        let versioned = VersionedDataset {
            metadata,
            content_hash,
            status,
        };
        self.datasets
            .insert(versioned.metadata.dataset_id.clone(), versioned.clone());
        Ok(versioned)
    }

    pub fn get(&self, dataset_id: &str) -> Option<&VersionedDataset> {
        self.datasets.get(dataset_id)
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
            if dataset.status == DatasetStatus::Degraded && !dataset.metadata.optional {
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
    require_text(&metadata.provider, "provider")?;
    require_text(&metadata.timezone, "timezone")?;
    require_text(&metadata.adjustment, "adjustment")?;
    require_text(&metadata.license, "license")?;
    require_text(&metadata.coverage.start, "coverage.start")?;
    require_text(&metadata.coverage.end, "coverage.end")?;
    require_text(&metadata.checksum, "checksum")?;
    require_text(&metadata.version, "version")?;
    if metadata.symbol_map.is_empty() {
        return Err(DataLakeError::MissingField("symbol_map"));
    }
    if metadata.coverage.expected_bars == 0 {
        return Err(DataLakeError::MissingField("coverage.expected_bars"));
    }
    if metadata.gaps.expected_bars == 0 {
        return Err(DataLakeError::MissingField("gaps.expected_bars"));
    }
    Ok(())
}

pub fn status_from_gaps(gaps: &GapSummary) -> DatasetStatus {
    if gaps.gap_percent() > 0.01 {
        DatasetStatus::Degraded
    } else {
        DatasetStatus::Healthy
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
mod tests {
    use super::*;
    use crate::spec_registry::{Instrument, StrategySpec};

    fn metadata(data_type: DataType) -> DatasetMetadata {
        DatasetMetadata {
            dataset_id: format!("{:?}-v1", data_type),
            provider: "approved-provider".into(),
            data_type,
            symbol_map: BTreeMap::from([("SPY".into(), "SPY".into())]),
            timezone: "America/New_York".into(),
            adjustment: "split_and_dividend".into(),
            license: "licensed".into(),
            coverage: CoverageWindow {
                start: "2020-01-01".into(),
                end: "2024-01-01".into(),
                expected_bars: 100,
                observed_bars: 100,
            },
            gaps: GapSummary {
                missing_bars: 0,
                expected_bars: 100,
            },
            checksum: "abc123".into(),
            version: "v1".into(),
            optional: false,
        }
    }

    #[test]
    fn t_data_05_rejects_missing_required_metadata_field() {
        let mut missing = metadata(DataType::Ohlcv);
        missing.provider.clear();
        assert_eq!(
            validate_metadata(&missing),
            Err(DataLakeError::MissingField("provider"))
        );
    }

    #[test]
    fn t_data_06_gap_above_one_percent_is_degraded_and_blocks_promotion() {
        let mut registry = DatasetRegistry::default();
        let mut degraded = metadata(DataType::Ohlcv);
        degraded.gaps = GapSummary {
            missing_bars: 2,
            expected_bars: 100,
        };
        let versioned = registry.register(degraded).unwrap();
        assert_eq!(versioned.status, DatasetStatus::Degraded);
        assert_eq!(
            registry.promotion_ready(&[InstrumentClass::Crypto], false),
            Err(DataLakeError::DegradedDataset)
        );
    }

    #[test]
    fn ec_trl_07_enforces_mandatory_matrix_and_event_news() {
        let mut registry = DatasetRegistry::default();
        for data_type in [
            DataType::Ohlcv,
            DataType::CorporateActions,
            DataType::Fundamentals,
            DataType::IndexConstituents,
            DataType::News,
        ] {
            registry.register(metadata(data_type)).unwrap();
        }
        assert!(
            registry
                .promotion_ready(&[InstrumentClass::Equity], true)
                .is_ok()
        );
        assert_eq!(
            registry.promotion_ready(&[InstrumentClass::Future], false),
            Err(DataLakeError::MissingMandatoryData(vec![
                DataType::ContinuousContract,
                DataType::ContractSpecs,
            ]))
        );
    }

    #[test]
    fn fx_or_options_need_spec_amendment_before_advancing_past_idea() {
        let mut spec = StrategySpec {
            spec_f01_instrument_universe: Some(vec![Instrument {
                symbol: "EURUSD".into(),
                venue: "OTC".into(),
                asset_class: "fx".into(),
            }]),
            spec_f02_timeframe_session: None,
            spec_f03_market_regime_assumptions: None,
            spec_f04_data_dependencies: None,
            spec_f05_entry_exit_rules: None,
            spec_f06_indicator_formulas: None,
            spec_f07_position_sizing: None,
            spec_f08_stops: None,
            spec_f09_invalidation_rules: None,
            spec_f10_no_trade_conditions: None,
            spec_f11_cost_assumptions: None,
            spec_f12_benchmark: None,
            spec_f13_expected_failure_modes: None,
            spec_f14_data_quality_tolerances_ms: None,
            spec_f15_promotion_status: Some(PromotionStatus::Idea),
        };
        assert!(spec_can_advance_past_idea(&spec).is_ok());
        spec.spec_f15_promotion_status = Some(PromotionStatus::Research);
        assert_eq!(
            spec_can_advance_past_idea(&spec),
            Err(DataLakeError::FxOptionsNeedSpecAmendment)
        );
    }
}
