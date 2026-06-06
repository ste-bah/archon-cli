use crate::order_intent::{OrderIntent, TradingMode};
use crate::risk_governor::{AccountState, MarketState};
use crate::risk_policy::RiskPolicy;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum HaltAttribution {
    StrategyAttributable,
    MarketOrInfrastructure,
    PolicyOrBypass,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ControlId {
    ReqRisk001,
    ReqRisk002,
    ReqRisk003,
    ReqRisk004,
    ReqRisk005,
    ReqRisk006,
    ReqRisk006a,
    ReqRisk007,
    ReqRisk008,
    ReqRisk009,
    ReqRisk010,
    ReqRisk011,
    ReqRisk012,
    ReqRisk013,
    ReqRisk014,
    ReqRisk015,
    ReqRisk017,
    ReqRisk019,
    ReqFail004,
}

impl ControlId {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::ReqRisk001 => "REQ-RISK-001",
            Self::ReqRisk002 => "REQ-RISK-002",
            Self::ReqRisk003 => "REQ-RISK-003",
            Self::ReqRisk004 => "REQ-RISK-004",
            Self::ReqRisk005 => "REQ-RISK-005",
            Self::ReqRisk006 => "REQ-RISK-006",
            Self::ReqRisk006a => "REQ-RISK-006a",
            Self::ReqRisk007 => "REQ-RISK-007",
            Self::ReqRisk008 => "REQ-RISK-008",
            Self::ReqRisk009 => "REQ-RISK-009",
            Self::ReqRisk010 => "REQ-RISK-010",
            Self::ReqRisk011 => "REQ-RISK-011",
            Self::ReqRisk012 => "REQ-RISK-012",
            Self::ReqRisk013 => "REQ-RISK-013",
            Self::ReqRisk014 => "REQ-RISK-014",
            Self::ReqRisk015 => "REQ-RISK-015",
            Self::ReqRisk017 => "REQ-RISK-017",
            Self::ReqRisk019 => "REQ-RISK-019",
            Self::ReqFail004 => "REQ-FAIL-004",
        }
    }

    pub const fn attribution(self) -> HaltAttribution {
        match self {
            Self::ReqRisk004
            | Self::ReqRisk005
            | Self::ReqRisk006
            | Self::ReqRisk006a
            | Self::ReqRisk015 => HaltAttribution::StrategyAttributable,
            Self::ReqRisk011 | Self::ReqRisk012 | Self::ReqRisk013 | Self::ReqRisk014 => {
                HaltAttribution::MarketOrInfrastructure
            }
            _ => HaltAttribution::PolicyOrBypass,
        }
    }

    pub const fn terminal(self) -> bool {
        matches!(self, Self::ReqRisk006)
    }

    pub const fn recoverable(self) -> bool {
        matches!(self, Self::ReqRisk006a)
    }
}

pub const CONTROL_ORDER: [ControlId; 19] = [
    ControlId::ReqFail004,
    ControlId::ReqRisk017,
    ControlId::ReqRisk001,
    ControlId::ReqRisk002,
    ControlId::ReqRisk003,
    ControlId::ReqRisk007,
    ControlId::ReqRisk008,
    ControlId::ReqRisk019,
    ControlId::ReqRisk004,
    ControlId::ReqRisk005,
    ControlId::ReqRisk006,
    ControlId::ReqRisk006a,
    ControlId::ReqRisk009,
    ControlId::ReqRisk010,
    ControlId::ReqRisk011,
    ControlId::ReqRisk012,
    ControlId::ReqRisk013,
    ControlId::ReqRisk014,
    ControlId::ReqRisk015,
];

pub fn evaluate_control(
    control: ControlId,
    intent: &OrderIntent,
    mode: TradingMode,
    account: &AccountState,
    market: &MarketState,
    policy: &RiskPolicy,
    now_unix_ms: i64,
) -> bool {
    match control {
        ControlId::ReqFail004 => account.governor_available && policy.validate_hash(),
        ControlId::ReqRisk017 => {
            intent.governor_required && matches!(mode, TradingMode::Paper | TradingMode::LivePilot)
        }
        ControlId::ReqRisk001 => {
            pct(intent.notional(), capital(mode, account, policy))
                <= policy.thresholds.max_strategy_exposure_pct
        }
        ControlId::ReqRisk002 => {
            pct(
                account.gross_exposure + intent.notional(),
                capital(mode, account, policy),
            ) <= policy.thresholds.max_account_exposure_pct
        }
        ControlId::ReqRisk003 => {
            pct(
                (account.net_exposure + intent.signed_notional()).abs(),
                capital(mode, account, policy),
            ) <= policy.thresholds.max_symbol_concentration_pct
        }
        ControlId::ReqRisk007 => {
            account.correlated_exposure_pct <= policy.thresholds.max_correlated_exposure_pct
        }
        ControlId::ReqRisk008 => {
            pct(intent.notional(), capital(mode, account, policy))
                <= policy.thresholds.max_order_notional_pct
        }
        ControlId::ReqRisk019 => account.leverage_after <= policy.thresholds.max_leverage,
        ControlId::ReqRisk004 => account.daily_loss_pct < policy.thresholds.max_daily_loss_pct,
        ControlId::ReqRisk005 => {
            account.weekly_loss_pct < policy.thresholds.max_daily_loss_pct * 2.5
        }
        ControlId::ReqRisk006 => {
            account.strategy_drawdown_pct < policy.thresholds.max_strategy_drawdown_pct
        }
        ControlId::ReqRisk006a => {
            mode != TradingMode::LivePilot
                || account.pilot_drawdown_pct < policy.thresholds.pilot_max_drawdown_pct
        }
        ControlId::ReqRisk009 => {
            account.order_rate_per_min <= policy.thresholds.max_order_rate_per_min
                && account.message_to_fill_ratio <= 50.0
        }
        ControlId::ReqRisk010 => account
            .allowed_instruments
            .iter()
            .any(|item| item == &intent.instrument),
        ControlId::ReqRisk011 => !market.event_window_active,
        ControlId::ReqRisk012 => market.realized_vol <= market.median_vol * 3.0,
        ControlId::ReqRisk013 => {
            market.data_age_seconds <= policy.thresholds.stale_data_max_seconds
        }
        ControlId::ReqRisk014 => {
            market.broker_healthy
                && market.broker_last_seen_seconds
                    <= policy.thresholds.broker_health_timeout_seconds
        }
        ControlId::ReqRisk015 => account
            .cooldown_until_unix_ms
            .is_none_or(|until| now_unix_ms >= until),
    }
}

fn capital(mode: TradingMode, account: &AccountState, policy: &RiskPolicy) -> f64 {
    match mode {
        TradingMode::Paper => policy.capital.paper_sim_capital,
        TradingMode::LivePilot => account.equity.min(policy.capital.pilot_capital_max_usd),
    }
    .max(1.0)
}

fn pct(value: f64, base: f64) -> f64 {
    (value.abs() / base.max(1.0)) * 100.0
}
