use crate::TradingError;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum DataType {
    Ohlcv,
    Fundamentals,
    SecFilings,
    Macro,
    Options,
    News,
    CftcFinra,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum LicenseTier {
    Public,
    Licensed,
    ResearchOnly,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ProviderClass {
    PublicFree,
    PaidLicensed,
    Unofficial,
    BrokerLinked,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Provider {
    Edgar,
    Fred,
    Cftc,
    Finra,
    YFinance,
    Polygon,
    NasdaqDataLink,
    Intrinio,
    Alpaca,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderPolicy {
    pub provider: Provider,
    pub data_type: DataType,
    pub license_tier: LicenseTier,
    pub provider_class: ProviderClass,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EvidenceProviderFlag {
    pub provider: Provider,
    pub data_type: DataType,
    pub license_tier: LicenseTier,
    pub provider_class: ProviderClass,
    pub promotion_eligible: bool,
}

impl ProviderPolicy {
    pub const fn evidence_flag(self) -> EvidenceProviderFlag {
        EvidenceProviderFlag {
            provider: self.provider,
            data_type: self.data_type,
            license_tier: self.license_tier,
            provider_class: self.provider_class,
            promotion_eligible: !matches!(self.license_tier, LicenseTier::ResearchOnly),
        }
    }
}

pub fn lookup_provider(data_type: DataType, provider: Provider) -> Option<ProviderPolicy> {
    ALLOWLIST
        .iter()
        .copied()
        .find(|policy| policy.data_type == data_type && policy.provider == provider)
}

pub fn require_provider(
    data_type: DataType,
    provider: Provider,
) -> Result<ProviderPolicy, TradingError> {
    lookup_provider(data_type, provider).ok_or(TradingError::OpenBbNotAllowlisted)
}

pub fn is_allowlisted(data_type: DataType, provider: Provider) -> bool {
    lookup_provider(data_type, provider).is_some()
}

pub fn promotion_eligible(data_type: DataType, provider: Provider) -> bool {
    lookup_provider(data_type, provider)
        .map(|policy| policy.evidence_flag().promotion_eligible)
        .unwrap_or(false)
}

pub const fn allowlist() -> &'static [ProviderPolicy] {
    ALLOWLIST
}

const ALLOWLIST: &[ProviderPolicy] = &[
    public(DataType::SecFilings, Provider::Edgar),
    public(DataType::Macro, Provider::Fred),
    public(DataType::CftcFinra, Provider::Cftc),
    public(DataType::CftcFinra, Provider::Finra),
    licensed(DataType::Ohlcv, Provider::Polygon),
    licensed(DataType::Fundamentals, Provider::Intrinio),
    licensed(DataType::Options, Provider::NasdaqDataLink),
    licensed(DataType::News, Provider::NasdaqDataLink),
    broker_linked(DataType::Ohlcv, Provider::Alpaca),
    research_only(DataType::Ohlcv, Provider::YFinance),
    research_only(DataType::Fundamentals, Provider::YFinance),
];

const fn public(data_type: DataType, provider: Provider) -> ProviderPolicy {
    ProviderPolicy {
        provider,
        data_type,
        license_tier: LicenseTier::Public,
        provider_class: ProviderClass::PublicFree,
    }
}

const fn licensed(data_type: DataType, provider: Provider) -> ProviderPolicy {
    ProviderPolicy {
        provider,
        data_type,
        license_tier: LicenseTier::Licensed,
        provider_class: ProviderClass::PaidLicensed,
    }
}

const fn broker_linked(data_type: DataType, provider: Provider) -> ProviderPolicy {
    ProviderPolicy {
        provider,
        data_type,
        license_tier: LicenseTier::Licensed,
        provider_class: ProviderClass::BrokerLinked,
    }
}

const fn research_only(data_type: DataType, provider: Provider) -> ProviderPolicy {
    ProviderPolicy {
        provider,
        data_type,
        license_tier: LicenseTier::ResearchOnly,
        provider_class: ProviderClass::Unofficial,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn non_allowlisted_pairs_are_denied_fail_closed() {
        assert!(!is_allowlisted(DataType::Options, Provider::YFinance));
        assert_eq!(
            require_provider(DataType::News, Provider::Fred)
                .unwrap_err()
                .code(),
            "ERR-OPENBB-NOT-ALLOWLISTED"
        );
    }

    #[test]
    fn phase_one_public_entries_are_allowlisted() {
        let cases = [
            (DataType::SecFilings, Provider::Edgar),
            (DataType::Macro, Provider::Fred),
            (DataType::CftcFinra, Provider::Cftc),
            (DataType::CftcFinra, Provider::Finra),
        ];

        for (data_type, provider) in cases {
            let policy = require_provider(data_type, provider).unwrap();
            assert_eq!(policy.license_tier, LicenseTier::Public);
            assert_eq!(policy.provider_class, ProviderClass::PublicFree);
            assert!(policy.evidence_flag().promotion_eligible);
        }
    }

    #[test]
    fn research_only_tier_propagates_to_promotion_evidence() {
        let flag = require_provider(DataType::Ohlcv, Provider::YFinance)
            .unwrap()
            .evidence_flag();

        assert_eq!(flag.license_tier, LicenseTier::ResearchOnly);
        assert_eq!(flag.provider_class, ProviderClass::Unofficial);
        assert!(!flag.promotion_eligible);
        assert!(!promotion_eligible(
            DataType::Fundamentals,
            Provider::YFinance
        ));
    }

    #[test]
    fn provider_classes_remain_separated() {
        assert_eq!(
            require_provider(DataType::Ohlcv, Provider::Polygon)
                .unwrap()
                .provider_class,
            ProviderClass::PaidLicensed
        );
        assert_eq!(
            require_provider(DataType::Ohlcv, Provider::Alpaca)
                .unwrap()
                .provider_class,
            ProviderClass::BrokerLinked
        );
    }
}
