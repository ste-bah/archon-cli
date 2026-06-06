use crate::TradingError;
use crate::adapters::openbb_allowlist::{self, LicenseTier, Provider};
use crate::data_lake::{
    CoverageWindow, DataType as LakeDataType, DatasetMetadata, DatasetRegistry, GapSummary,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AccessMode {
    Research,
    LiveRequired,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum OpenBbRoute {
    Rest,
    Sdk,
    McpResearchOnly,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OpenBbRequest {
    pub provider: Provider,
    pub allowlist_data_type: openbb_allowlist::DataType,
    pub lake_data_type: LakeDataType,
    pub endpoint: String,
    pub params: BTreeMap<String, String>,
    pub creds_profile_ref: String,
    pub cache_key: String,
    pub schema_version: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OpenBbResponse {
    pub body: Vec<u8>,
    pub warnings: Vec<String>,
    pub metadata: BTreeMap<String, String>,
    pub quality: DataQuality,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DataQuality {
    pub complete: bool,
    pub licensed: bool,
    pub timestamp_fresh: bool,
    pub survivorship_adjusted: bool,
    pub corporate_actions_adjusted: bool,
    pub reproducible: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OpenBbProvenance {
    pub provider: Provider,
    pub endpoint: String,
    pub params: BTreeMap<String, String>,
    pub meta: BTreeMap<String, String>,
    pub warnings: Vec<String>,
    pub creds_profile_ref: String,
    pub timestamp: String,
    pub checksum: String,
    pub schema_version: String,
    pub route: OpenBbRoute,
    pub license_tier: LicenseTier,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GovernedDataset {
    pub provenance: OpenBbProvenance,
    pub metadata: DatasetMetadata,
    pub body: Vec<u8>,
    pub promotion_eligible: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OpenBbError {
    NotAllowlisted,
    DiscoveryUnavailable,
    RateLimited,
    CacheMissLiveRequired,
    GateFailed(&'static str),
    SecretMaterialRejected,
    TransportUnavailable,
    LakeRejected,
}

impl OpenBbError {
    pub const fn code(&self) -> &'static str {
        match self {
            Self::NotAllowlisted => "ERR-OPENBB-NOT-ALLOWLISTED",
            Self::DiscoveryUnavailable => "ERR-OPENBB-DISCOVERY-UNAVAILABLE",
            Self::RateLimited => "ERR-OPENBB-RATE-LIMITED",
            Self::CacheMissLiveRequired => "ERR-OPENBB-LIVE-CACHE-MISS",
            Self::GateFailed(gate) => gate,
            Self::SecretMaterialRejected => "ERR-OPENBB-SECRET-MATERIAL",
            Self::TransportUnavailable => "ERR-OPENBB-TRANSPORT-UNAVAILABLE",
            Self::LakeRejected => "ERR-OPENBB-LAKE-REJECTED",
        }
    }
}

impl From<TradingError> for OpenBbError {
    fn from(error: TradingError) -> Self {
        match error {
            TradingError::OpenBbNotAllowlisted => Self::NotAllowlisted,
            _ => Self::GateFailed("ERR-OPENBB-GATE"),
        }
    }
}

pub trait OpenBbTransport {
    fn discover(&self, request: &OpenBbRequest) -> bool;
    fn rest(&mut self, request: &OpenBbRequest) -> Result<OpenBbResponse, OpenBbError>;
    fn sdk(&mut self, request: &OpenBbRequest) -> Result<OpenBbResponse, OpenBbError>;
    fn mcp(&mut self, request: &OpenBbRequest) -> Result<OpenBbResponse, OpenBbError>;
}

#[derive(Debug, Default, Clone)]
pub struct OpenBbGateway {
    cache: BTreeMap<String, GovernedDataset>,
    provenance_log: Vec<OpenBbProvenance>,
    lake_registry: DatasetRegistry,
}

impl OpenBbGateway {
    pub fn fetch<T: OpenBbTransport>(
        &mut self,
        transport: &mut T,
        request: OpenBbRequest,
        mode: AccessMode,
        timestamp: impl Into<String>,
    ) -> Result<GovernedDataset, OpenBbError> {
        let policy =
            openbb_allowlist::require_provider(request.allowlist_data_type, request.provider)?;
        reject_secret_material(&request.creds_profile_ref)?;
        if !transport.discover(&request) {
            return self.cache_or_live_fail(&request, mode);
        }
        match self.fetch_uncached(
            transport,
            &request,
            policy.license_tier,
            mode,
            timestamp.into(),
        ) {
            Ok(dataset) => Ok(dataset),
            Err(OpenBbError::RateLimited) => self.cache_or_live_fail(&request, mode),
            Err(error) => Err(error),
        }
    }

    pub fn provenance_log(&self) -> &[OpenBbProvenance] {
        &self.provenance_log
    }

    pub fn lake_registry(&self) -> &DatasetRegistry {
        &self.lake_registry
    }

    fn fetch_uncached<T: OpenBbTransport>(
        &mut self,
        transport: &mut T,
        request: &OpenBbRequest,
        license_tier: LicenseTier,
        mode: AccessMode,
        timestamp: String,
    ) -> Result<GovernedDataset, OpenBbError> {
        let (route, response) = priority_fetch(transport, request, mode)?;
        enforce_gates(&response.quality, mode)?;
        let provenance = provenance(request, &response, route, license_tier, timestamp);
        self.provenance_log.push(provenance.clone());
        let dataset = self.persist_to_lake(request, response, provenance)?;
        self.cache
            .insert(request.cache_key.clone(), dataset.clone());
        Ok(dataset)
    }

    fn cache_or_live_fail(
        &self,
        request: &OpenBbRequest,
        mode: AccessMode,
    ) -> Result<GovernedDataset, OpenBbError> {
        match self.cache.get(&request.cache_key) {
            Some(dataset) => Ok(dataset.clone()),
            None if mode == AccessMode::LiveRequired => Err(OpenBbError::CacheMissLiveRequired),
            None => Err(OpenBbError::DiscoveryUnavailable),
        }
    }

    fn persist_to_lake(
        &mut self,
        request: &OpenBbRequest,
        response: OpenBbResponse,
        provenance: OpenBbProvenance,
    ) -> Result<GovernedDataset, OpenBbError> {
        let metadata = dataset_metadata(request, &response, &provenance);
        self.lake_registry
            .register(metadata.clone())
            .map_err(|_| OpenBbError::LakeRejected)?;
        Ok(GovernedDataset {
            promotion_eligible: provenance.license_tier != LicenseTier::ResearchOnly,
            provenance,
            metadata,
            body: response.body,
        })
    }
}

fn priority_fetch<T: OpenBbTransport>(
    transport: &mut T,
    request: &OpenBbRequest,
    mode: AccessMode,
) -> Result<(OpenBbRoute, OpenBbResponse), OpenBbError> {
    match transport.rest(request) {
        Ok(response) => Ok((OpenBbRoute::Rest, response)),
        Err(OpenBbError::RateLimited) => Err(OpenBbError::RateLimited),
        Err(_) => match transport.sdk(request) {
            Ok(response) => Ok((OpenBbRoute::Sdk, response)),
            Err(_) if mode == AccessMode::Research => transport
                .mcp(request)
                .map(|response| (OpenBbRoute::McpResearchOnly, response)),
            Err(_) => Err(OpenBbError::TransportUnavailable),
        },
    }
}

fn enforce_gates(quality: &DataQuality, mode: AccessMode) -> Result<(), OpenBbError> {
    let failed = if !quality.complete {
        Some("ERR-OPENBB-INCOMPLETE")
    } else if !quality.licensed {
        Some("ERR-OPENBB-UNLICENSED")
    } else if !quality.timestamp_fresh {
        Some("ERR-OPENBB-STALE")
    } else if !quality.survivorship_adjusted {
        Some("ERR-OPENBB-SURVIVORSHIP")
    } else if !quality.corporate_actions_adjusted {
        Some("ERR-OPENBB-CORPORATE-ACTIONS")
    } else if !quality.reproducible {
        Some("ERR-OPENBB-NON-REPRODUCIBLE")
    } else {
        None
    };
    match (failed, mode) {
        (Some(gate), AccessMode::LiveRequired) => Err(OpenBbError::GateFailed(gate)),
        (Some("ERR-OPENBB-UNLICENSED"), AccessMode::Research) => {
            Err(OpenBbError::GateFailed("ERR-OPENBB-UNLICENSED"))
        }
        _ => Ok(()),
    }
}

fn provenance(
    request: &OpenBbRequest,
    response: &OpenBbResponse,
    route: OpenBbRoute,
    license_tier: LicenseTier,
    timestamp: String,
) -> OpenBbProvenance {
    OpenBbProvenance {
        provider: request.provider,
        endpoint: request.endpoint.clone(),
        params: request.params.clone(),
        meta: response.metadata.clone(),
        warnings: response.warnings.clone(),
        creds_profile_ref: request.creds_profile_ref.clone(),
        timestamp,
        checksum: blake3::hash(&response.body).to_hex().to_string(),
        schema_version: request.schema_version.clone(),
        route,
        license_tier,
    }
}

fn dataset_metadata(
    request: &OpenBbRequest,
    response: &OpenBbResponse,
    provenance: &OpenBbProvenance,
) -> DatasetMetadata {
    DatasetMetadata {
        dataset_id: request.cache_key.clone(),
        provider: format!("{:?}", request.provider),
        data_type: request.lake_data_type,
        symbol_map: BTreeMap::from([(
            metadata_or(&response.metadata, "symbol", "UNKNOWN"),
            metadata_or(&response.metadata, "provider_symbol", "UNKNOWN"),
        )]),
        timezone: metadata_or(&response.metadata, "timezone", "UTC"),
        adjustment: metadata_or(&response.metadata, "adjustment", "declared"),
        license: format!("{:?}", provenance.license_tier),
        coverage: CoverageWindow {
            start: metadata_or(&response.metadata, "coverage_start", "unknown-start"),
            end: metadata_or(&response.metadata, "coverage_end", "unknown-end"),
            expected_bars: parse_u64(response.metadata.get("expected_bars"), 1),
            observed_bars: parse_u64(response.metadata.get("observed_bars"), 1),
        },
        gaps: GapSummary {
            missing_bars: parse_u64(response.metadata.get("missing_bars"), 0),
            expected_bars: parse_u64(response.metadata.get("expected_bars"), 1),
        },
        checksum: provenance.checksum.clone(),
        version: request.schema_version.clone(),
        optional: false,
    }
}

fn metadata_or(metadata: &BTreeMap<String, String>, key: &str, fallback: &str) -> String {
    metadata
        .get(key)
        .cloned()
        .unwrap_or_else(|| fallback.into())
}

fn parse_u64(value: Option<&String>, fallback: u64) -> u64 {
    value.and_then(|text| text.parse().ok()).unwrap_or(fallback)
}

fn reject_secret_material(creds_profile_ref: &str) -> Result<(), OpenBbError> {
    let lower = creds_profile_ref.to_ascii_lowercase();
    let looks_secret = lower.contains("secret=")
        || lower.contains("token=")
        || lower.contains("apikey=")
        || lower.contains("api_key=")
        || creds_profile_ref.len() > 128;
    if creds_profile_ref.trim().is_empty() || looks_secret {
        Err(OpenBbError::SecretMaterialRejected)
    } else {
        Ok(())
    }
}

#[cfg(test)]
#[path = "openbb_tests.rs"]
mod tests;
