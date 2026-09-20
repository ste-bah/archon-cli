//! The persisted dataset schema — every type that is serialised into
//! `metadata.json`, `manifest.json` and `registry.json`.
//!
//! Split out of `data_lake.rs` so the parent reads as *rules* (registry
//! behaviour, metadata validation, provider capability) rather than opening
//! with two hundred lines of field declarations. Nothing in here decides
//! anything: it only describes what a dataset record looks like on disk, plus
//! the serde defaults that fill in fields older artifacts never wrote.

use serde::{Deserialize, Deserializer, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum InstrumentClass {
    Equity,
    Crypto,
    Future,
    Fx,
    Option,
}

/// Kind of data a dataset holds.
///
/// Read case-insensitively; still written in PascalCase.
///
/// Agents author `metadata.json` and `manifest.json` by hand, and `ohlcv` is a
/// more natural spelling than `Ohlcv` for something the rest of the tree writes
/// lowercase everywhere else — filenames, dataset ids, CLI arguments. Five
/// artifacts on one installation used it, and because the registry is loaded as
/// a unit, a single one of them made the WHOLE lake unreadable: `archon trading
/// data status` and `list` both failed outright on a casing difference in one
/// field of one file.
///
/// The variants themselves are a closed vocabulary and stay strict — an
/// unrecognised *kind* is a real error and must still fail. Only case is
/// forgiven, which cannot make one kind masquerade as another.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
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

impl DataType {
    /// All variants, in declaration order, for parsing and error text.
    const ALL: &'static [(&'static str, DataType)] = &[
        ("ohlcv", DataType::Ohlcv),
        ("corporateactions", DataType::CorporateActions),
        ("fundamentals", DataType::Fundamentals),
        ("borrow", DataType::Borrow),
        ("funding", DataType::Funding),
        ("indexconstituents", DataType::IndexConstituents),
        ("continuouscontract", DataType::ContinuousContract),
        ("contractspecs", DataType::ContractSpecs),
        ("news", DataType::News),
        ("tick", DataType::Tick),
        ("orderbook", DataType::OrderBook),
    ];
}

impl<'de> Deserialize<'de> for DataType {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let raw = String::deserialize(deserializer)?;
        // Compare with separators stripped so `corporate_actions`,
        // `corporate-actions` and `CorporateActions` all agree. This normalises
        // spelling, not meaning: an unknown kind still errors below.
        let normalized: String = raw
            .chars()
            .filter(|c| c.is_ascii_alphanumeric())
            .map(|c| c.to_ascii_lowercase())
            .collect();
        Self::ALL
            .iter()
            .find(|(name, _)| *name == normalized)
            .map(|(_, variant)| *variant)
            .ok_or_else(|| {
                serde::de::Error::custom(format!(
                    "unknown data_type `{raw}`; expected one of: {}",
                    Self::ALL
                        .iter()
                        .map(|(name, _)| *name)
                        .collect::<Vec<_>>()
                        .join(", ")
                ))
            })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DatasetStatus {
    Healthy,
    #[serde(alias = "Quarantined")]
    Degraded,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CoverageWindow {
    pub start: String,
    pub end: String,
    pub expected_bars: u64,
    pub observed_bars: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
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

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DatasetChecksums {
    #[serde(default)]
    pub raw_sha256: String,
    #[serde(default)]
    pub normalized_sha256: String,
    #[serde(default)]
    pub metadata_sha256: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DatasetArtifactPaths {
    #[serde(default)]
    pub raw: String,
    #[serde(default)]
    pub raw_response: String,
    #[serde(default)]
    pub raw_request: String,
    #[serde(default)]
    pub redacted_headers: String,
    #[serde(default)]
    pub provider_notes: String,
    #[serde(default)]
    pub normalized: String,
    #[serde(default)]
    pub validation: String,
    #[serde(default)]
    pub manifest: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DatasetSourceMetadata {
    #[serde(default)]
    pub license_notes: String,
    #[serde(default)]
    pub url_or_endpoint: String,
    #[serde(default)]
    pub retrieved_at: String,
    #[serde(default)]
    pub credential_required: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DatasetMetadata {
    #[serde(default = "dataset_schema")]
    #[serde(rename = "schema", alias = "schema_version")]
    pub schema_version: String,
    pub dataset_id: String,
    pub version: String,
    #[serde(default)]
    pub canonical_instrument: String,
    #[serde(default)]
    pub asset_class: String,
    pub provider: String,
    #[serde(default)]
    pub provider_symbol: String,
    #[serde(default)]
    pub timeframe: String,
    #[serde(default)]
    pub native_interval: bool,
    #[serde(default)]
    pub production_eligible: bool,
    #[serde(default = "raw_basis")]
    pub price_basis: String,
    #[serde(default = "provider_default_session")]
    pub session: String,
    pub data_type: DataType,
    pub symbol_map: BTreeMap<String, String>,
    pub timezone: String,
    pub adjustment: String,
    pub license: String,
    pub coverage: CoverageWindow,
    pub gaps: GapSummary,
    pub checksum: String,
    #[serde(default)]
    pub checksums: DatasetChecksums,
    #[serde(default)]
    pub paths: DatasetArtifactPaths,
    #[serde(default)]
    pub source: DatasetSourceMetadata,
    #[serde(default = "degraded_quality")]
    pub quality_status: String,
    #[serde(default)]
    pub created_at: String,
    pub optional: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VersionedDataset {
    pub metadata: DatasetMetadata,
    pub content_hash: String,
    pub status: DatasetStatus,
}

/// Not private: the `metadata()` fixture in `data_lake/tests.rs` builds a
/// `DatasetMetadata` field by field and must stamp the same schema string
/// serde would have defaulted in, or the fixture drifts from what is written
/// to disk.
pub(super) fn dataset_schema() -> String {
    "archon-trading-dataset-v2".into()
}

fn raw_basis() -> String {
    "raw".into()
}

fn provider_default_session() -> String {
    "provider_default".into()
}

fn degraded_quality() -> String {
    "degraded".into()
}
