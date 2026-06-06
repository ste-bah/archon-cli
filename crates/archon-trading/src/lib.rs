//! Provider-neutral Trading Research & Execution Lab primitives.
//!
//! This crate contains the TRL workstream modules while preserving the shared
//! provider-neutral error-code contract below.

pub mod adapters {
    pub mod broker;
    pub mod openbb;
    pub mod openbb_allowlist;
    pub mod tv_mcp;
}

pub mod agent_policy;
pub mod audit_ledger;
pub mod backtest;
pub mod data_lake;
pub mod dryrun_cert;
pub mod kb;
pub mod kill_switch;
pub mod learning_hooks;
pub mod live_enablement;
pub mod live_terminal;
pub mod maker_checker;
pub mod order_intent;
pub mod paper_terminal;
pub mod pine_lab;
pub mod postmortem;
pub mod promotion;
pub mod regime;
pub mod risk_controls;
pub mod risk_governor;
pub mod risk_policy;
pub mod spec_registry;

/// Stable provider-neutral error surface for trading workflows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TradingError {
    AuditChainBroken,
    BrokerReject,
    InvalidSpec,
    LivePhase5Prereq,
    OpenBbNotAllowlisted,
    OrderTypeUnsupported,
    PineCrossSymbol,
    PolicyDenied,
    RiskReject(&'static str),
}

impl TradingError {
    /// Return a stable machine-readable error code.
    pub const fn code(&self) -> &'static str {
        match self {
            Self::AuditChainBroken => "ERR-AUDIT-CHAIN-BROKEN",
            Self::BrokerReject => "BROKER_REJECT",
            Self::InvalidSpec => "ERR-SPEC-INVALID",
            Self::LivePhase5Prereq => "ERR-LIVE-PHASE5-PREREQ",
            Self::OpenBbNotAllowlisted => "ERR-OPENBB-NOT-ALLOWLISTED",
            Self::OrderTypeUnsupported => "ORDER_TYPE_UNSUPPORTED",
            Self::PineCrossSymbol => "ERR-PINE-CROSS-SYMBOL",
            Self::PolicyDenied => "ERR-POLICY-DENIED",
            Self::RiskReject(code) => code,
        }
    }
}

impl std::fmt::Display for TradingError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::RiskReject(control) => write!(formatter, "risk rejected by {control}"),
            other => formatter.write_str(other.code()),
        }
    }
}

impl std::error::Error for TradingError {}

#[cfg(test)]
mod tests {
    use super::TradingError;

    #[test]
    fn trading_error_codes_are_stable() {
        let cases = [
            (TradingError::AuditChainBroken, "ERR-AUDIT-CHAIN-BROKEN"),
            (TradingError::BrokerReject, "BROKER_REJECT"),
            (TradingError::InvalidSpec, "ERR-SPEC-INVALID"),
            (TradingError::LivePhase5Prereq, "ERR-LIVE-PHASE5-PREREQ"),
            (
                TradingError::OpenBbNotAllowlisted,
                "ERR-OPENBB-NOT-ALLOWLISTED",
            ),
            (TradingError::OrderTypeUnsupported, "ORDER_TYPE_UNSUPPORTED"),
            (TradingError::PineCrossSymbol, "ERR-PINE-CROSS-SYMBOL"),
            (TradingError::PolicyDenied, "ERR-POLICY-DENIED"),
            (
                TradingError::RiskReject("RISK-REJECT-REQ-RISK-001"),
                "RISK-REJECT-REQ-RISK-001",
            ),
        ];

        for (error, code) in cases {
            assert_eq!(error.code(), code);
        }
    }
}
