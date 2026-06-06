use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Instrument {
    pub symbol: String,
    pub venue: String,
    pub asset_class: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TimeSession {
    pub timeframe: String,
    pub session_hours: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DatasetRef {
    pub dataset_id: String,
    pub version: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuleSet {
    pub rules: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FormulaSet {
    pub formulas: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PositionSizing {
    pub model: String,
    pub max_risk_pct: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SpecF08Stops {
    pub stop_rules: Vec<String>,
    pub take_profit_rules: Vec<String>,
    pub trailing_rules: Vec<String>,
    pub max_strategy_drawdown_pct: f64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CostModel {
    pub slippage_bps: u32,
    pub fee_bps: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BenchmarkRef {
    pub symbol: String,
    pub source: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum PromotionStatus {
    Idea,
    Research,
    Backtest,
    Paper,
    LivePilot,
    Retired,
}

impl PromotionStatus {
    pub const fn next(self) -> Option<Self> {
        match self {
            Self::Idea => Some(Self::Research),
            Self::Research => Some(Self::Backtest),
            Self::Backtest => Some(Self::Paper),
            Self::Paper => Some(Self::LivePilot),
            Self::LivePilot => Some(Self::Retired),
            Self::Retired => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SpecRegistryError {
    MissingOrInvalidFields(Vec<&'static str>),
    NoDrawdownCeiling,
    StatusSkip,
    RetiredTerminal,
    ImmutableReferencedVersion,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StrategySpec {
    pub spec_f01_instrument_universe: Option<Vec<Instrument>>,
    pub spec_f02_timeframe_session: Option<TimeSession>,
    pub spec_f03_market_regime_assumptions: Option<Vec<String>>,
    pub spec_f04_data_dependencies: Option<Vec<DatasetRef>>,
    pub spec_f05_entry_exit_rules: Option<RuleSet>,
    pub spec_f06_indicator_formulas: Option<FormulaSet>,
    pub spec_f07_position_sizing: Option<PositionSizing>,
    pub spec_f08_stops: Option<SpecF08Stops>,
    pub spec_f09_invalidation_rules: Option<RuleSet>,
    pub spec_f10_no_trade_conditions: Option<RuleSet>,
    pub spec_f11_cost_assumptions: Option<CostModel>,
    pub spec_f12_benchmark: Option<BenchmarkRef>,
    pub spec_f13_expected_failure_modes: Option<Vec<String>>,
    pub spec_f14_data_quality_tolerances_ms: Option<BTreeMap<String, u64>>,
    pub spec_f15_promotion_status: Option<PromotionStatus>,
}

impl StrategySpec {
    pub fn validate(&self) -> Vec<&'static str> {
        let mut invalid = Vec::new();
        self.require(
            self.spec_f01_instrument_universe.is_some(),
            "SPEC-F01",
            &mut invalid,
        );
        self.require(
            self.spec_f02_timeframe_session.is_some(),
            "SPEC-F02",
            &mut invalid,
        );
        self.require(
            self.spec_f03_market_regime_assumptions.is_some(),
            "SPEC-F03",
            &mut invalid,
        );
        self.require(
            self.spec_f04_data_dependencies.is_some(),
            "SPEC-F04",
            &mut invalid,
        );
        self.require(
            self.spec_f05_entry_exit_rules.is_some(),
            "SPEC-F05",
            &mut invalid,
        );
        self.require(
            self.spec_f06_indicator_formulas.is_some(),
            "SPEC-F06",
            &mut invalid,
        );
        self.require(
            self.spec_f07_position_sizing.is_some(),
            "SPEC-F07",
            &mut invalid,
        );
        self.require(
            valid_stops(self.spec_f08_stops.as_ref()),
            "SPEC-F08",
            &mut invalid,
        );
        self.require(
            self.spec_f09_invalidation_rules.is_some(),
            "SPEC-F09",
            &mut invalid,
        );
        self.require(
            self.spec_f10_no_trade_conditions.is_some(),
            "SPEC-F10",
            &mut invalid,
        );
        self.require(
            self.spec_f11_cost_assumptions.is_some(),
            "SPEC-F11",
            &mut invalid,
        );
        self.require(self.spec_f12_benchmark.is_some(), "SPEC-F12", &mut invalid);
        self.require(
            self.spec_f13_expected_failure_modes.is_some(),
            "SPEC-F13",
            &mut invalid,
        );
        self.require(
            self.spec_f14_data_quality_tolerances_ms.is_some(),
            "SPEC-F14",
            &mut invalid,
        );
        self.require(
            self.spec_f15_promotion_status.is_some(),
            "SPEC-F15",
            &mut invalid,
        );
        invalid
    }

    pub fn validated(&self) -> Result<(), SpecRegistryError> {
        let invalid = self.validate();
        if invalid.is_empty() {
            Ok(())
        } else if invalid == ["SPEC-F08"] && self.spec_f08_stops.is_some() {
            Err(SpecRegistryError::NoDrawdownCeiling)
        } else {
            Err(SpecRegistryError::MissingOrInvalidFields(invalid))
        }
    }

    pub fn content_hash(&self) -> Result<String, SpecRegistryError> {
        self.validated()?;
        let bytes = serde_json::to_vec(self)
            .map_err(|_| SpecRegistryError::MissingOrInvalidFields(vec!["SPEC-FORMAT"]))?;
        Ok(blake3::hash(&bytes).to_hex().to_string())
    }

    pub fn advance_status(&self, target: PromotionStatus) -> Result<Self, SpecRegistryError> {
        self.validated()?;
        let current = self
            .spec_f15_promotion_status
            .expect("validated status present");
        if current == PromotionStatus::Retired {
            return Err(SpecRegistryError::RetiredTerminal);
        }
        if current.next() != Some(target) {
            return Err(SpecRegistryError::StatusSkip);
        }
        let mut advanced = self.clone();
        advanced.spec_f15_promotion_status = Some(target);
        Ok(advanced)
    }

    pub fn pine_omission_reason(&self) -> Option<&'static str> {
        match &self.spec_f01_instrument_universe {
            Some(instruments) if instruments.is_empty() => Some("OMIT_NO_TRADABLE_RULES"),
            _ => None,
        }
    }

    fn require(&self, condition: bool, field: &'static str, invalid: &mut Vec<&'static str>) {
        if !condition {
            invalid.push(field);
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VersionedStrategySpec {
    pub spec: StrategySpec,
    pub content_hash: String,
    pub evidence_referenced: bool,
}

impl VersionedStrategySpec {
    pub fn new(spec: StrategySpec) -> Result<Self, SpecRegistryError> {
        let content_hash = spec.content_hash()?;
        Ok(Self {
            spec,
            content_hash,
            evidence_referenced: false,
        })
    }

    pub fn mark_evidence_referenced(&mut self) {
        self.evidence_referenced = true;
    }

    pub fn replace_spec(&mut self, spec: StrategySpec) -> Result<(), SpecRegistryError> {
        if self.evidence_referenced {
            return Err(SpecRegistryError::ImmutableReferencedVersion);
        }
        self.content_hash = spec.content_hash()?;
        self.spec = spec;
        Ok(())
    }

    pub fn verify_checksum(&self) -> bool {
        self.spec
            .content_hash()
            .is_ok_and(|hash| hash == self.content_hash)
    }
}

pub fn parse_strategy_spec_json(input: &str) -> Result<StrategySpec, SpecRegistryError> {
    serde_json::from_str::<StrategySpec>(input)
        .map_err(|_| SpecRegistryError::MissingOrInvalidFields(vec!["SPEC-TYPE"]))
}

fn valid_stops(stops: Option<&SpecF08Stops>) -> bool {
    stops.is_some_and(|value| {
        value.max_strategy_drawdown_pct.is_finite() && value.max_strategy_drawdown_pct > 0.0
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn full_spec() -> StrategySpec {
        StrategySpec {
            spec_f01_instrument_universe: Some(vec![Instrument {
                symbol: "SPY".into(),
                venue: "ARCA".into(),
                asset_class: "equity".into(),
            }]),
            spec_f02_timeframe_session: Some(TimeSession {
                timeframe: "1D".into(),
                session_hours: "regular".into(),
            }),
            spec_f03_market_regime_assumptions: Some(vec!["baseline".into()]),
            spec_f04_data_dependencies: Some(vec![DatasetRef {
                dataset_id: "ohlcv".into(),
                version: "v1".into(),
            }]),
            spec_f05_entry_exit_rules: Some(RuleSet {
                rules: vec!["close > sma20".into()],
            }),
            spec_f06_indicator_formulas: Some(FormulaSet {
                formulas: vec!["sma(close,20)".into()],
            }),
            spec_f07_position_sizing: Some(PositionSizing {
                model: "fixed_fractional".into(),
                max_risk_pct: "1".into(),
            }),
            spec_f08_stops: Some(SpecF08Stops {
                stop_rules: vec!["2atr".into()],
                take_profit_rules: vec!["3atr".into()],
                trailing_rules: vec![],
                max_strategy_drawdown_pct: 8.0,
            }),
            spec_f09_invalidation_rules: Some(RuleSet {
                rules: vec!["vol regime shift".into()],
            }),
            spec_f10_no_trade_conditions: Some(RuleSet {
                rules: vec!["high impact event".into()],
            }),
            spec_f11_cost_assumptions: Some(CostModel {
                slippage_bps: 2,
                fee_bps: 1,
            }),
            spec_f12_benchmark: Some(BenchmarkRef {
                symbol: "SPY".into(),
                source: "approved".into(),
            }),
            spec_f13_expected_failure_modes: Some(vec!["gap risk".into()]),
            spec_f14_data_quality_tolerances_ms: Some(BTreeMap::from([("ohlcv".into(), 5000)])),
            spec_f15_promotion_status: Some(PromotionStatus::Idea),
        }
    }

    #[test]
    fn full_15_field_spec_is_valid_and_content_addressed() {
        let versioned = VersionedStrategySpec::new(full_spec()).unwrap();
        assert_eq!(versioned.content_hash.len(), 64);
        assert!(versioned.verify_checksum());
    }

    #[test]
    fn missing_field_rejection_names_exact_spec_id() {
        let mut spec = full_spec();
        spec.spec_f12_benchmark = None;
        assert_eq!(spec.validate(), vec!["SPEC-F12"]);
    }

    #[test]
    fn missing_drawdown_ceiling_rejects_spec_f08() {
        let mut spec = full_spec();
        spec.spec_f08_stops
            .as_mut()
            .unwrap()
            .max_strategy_drawdown_pct = 0.0;
        assert_eq!(spec.validated(), Err(SpecRegistryError::NoDrawdownCeiling));
    }

    #[test]
    fn promotion_is_one_step_and_retired_is_terminal() {
        let research = full_spec()
            .advance_status(PromotionStatus::Research)
            .unwrap();
        assert_eq!(
            research.spec_f15_promotion_status,
            Some(PromotionStatus::Research)
        );
        assert_eq!(
            full_spec().advance_status(PromotionStatus::Backtest),
            Err(SpecRegistryError::StatusSkip)
        );
        let mut retired = full_spec();
        retired.spec_f15_promotion_status = Some(PromotionStatus::Retired);
        assert_eq!(
            retired.advance_status(PromotionStatus::Retired),
            Err(SpecRegistryError::RetiredTerminal)
        );
    }

    #[test]
    fn empty_instrument_universe_drives_pine_omission_reason() {
        let mut spec = full_spec();
        spec.spec_f01_instrument_universe = Some(vec![]);
        assert_eq!(spec.pine_omission_reason(), Some("OMIT_NO_TRADABLE_RULES"));
        assert!(spec.validated().is_ok());
    }

    #[test]
    fn referenced_version_is_immutable_and_type_coercion_fails() {
        let mut versioned = VersionedStrategySpec::new(full_spec()).unwrap();
        versioned.mark_evidence_referenced();
        assert_eq!(
            versioned.replace_spec(full_spec()),
            Err(SpecRegistryError::ImmutableReferencedVersion)
        );
        let json = serde_json::to_string(&full_spec()).unwrap().replace(
            "\"max_strategy_drawdown_pct\":8.0",
            "\"max_strategy_drawdown_pct\":\"8.0\"",
        );
        assert!(parse_strategy_spec_json(&json).is_err());
    }
}
