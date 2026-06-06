use crate::TradingError;
use crate::pine_lab::{GeneratedPineScript, ScriptVariant};
use crate::risk_governor::{
    AccountState, MarketState, RiskDecision, RiskGovernor, RiskGovernorError,
};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TradingMode {
    Paper,
    LivePilot,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum OrderSide {
    Buy,
    Sell,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum OrderType {
    Market,
    Limit,
    Stop,
    StopLimit,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TimeInForce {
    Day,
    Gtc,
    Ioc,
    Fok,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct OrderPrices {
    pub reference: f64,
    pub limit: Option<f64>,
    pub stop: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OrderIntent {
    pub strategy_id: String,
    pub instrument: String,
    pub side: OrderSide,
    pub order_type: OrderType,
    pub quantity: f64,
    pub prices: OrderPrices,
    pub tif: TimeInForce,
    pub mode: TradingMode,
    pub governor_required: bool,
    pub source: IntentSource,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum IntentSource {
    Manual,
    PineAlert { script_id: String },
    System { component: String },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PineAlertPayload {
    pub strategy_id: String,
    pub instrument: String,
    pub side: OrderSide,
    pub order_type: OrderType,
    pub quantity: f64,
    pub prices: OrderPrices,
    pub tif: TimeInForce,
    pub mode: TradingMode,
    pub script_id: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct GatedOrderIntent {
    pub intent: OrderIntent,
    pub decision: RiskDecision,
}

impl OrderIntent {
    pub fn new(
        strategy_id: impl Into<String>,
        instrument: impl Into<String>,
        side: OrderSide,
        order_type: OrderType,
        quantity: f64,
        prices: OrderPrices,
        mode: TradingMode,
    ) -> Self {
        Self {
            strategy_id: strategy_id.into(),
            instrument: instrument.into(),
            side,
            order_type,
            quantity,
            prices,
            tif: TimeInForce::Day,
            mode,
            governor_required: true,
            source: IntentSource::Manual,
        }
    }

    pub fn notional(&self) -> f64 {
        self.quantity.abs() * self.execution_price()
    }

    pub fn signed_notional(&self) -> f64 {
        match self.side {
            OrderSide::Buy => self.notional(),
            OrderSide::Sell => -self.notional(),
        }
    }

    pub fn execution_price(&self) -> f64 {
        match self.order_type {
            OrderType::Market => self.prices.reference,
            OrderType::Limit => self.prices.limit.unwrap_or(self.prices.reference),
            OrderType::Stop => self.prices.stop.unwrap_or(self.prices.reference),
            OrderType::StopLimit => self
                .prices
                .limit
                .or(self.prices.stop)
                .unwrap_or(self.prices.reference),
        }
    }

    pub fn with_tif(mut self, tif: TimeInForce) -> Self {
        self.tif = tif;
        self
    }
}

impl Default for OrderIntent {
    fn default() -> Self {
        Self::new(
            "strategy-a",
            "SPY",
            OrderSide::Buy,
            OrderType::Market,
            1.0,
            OrderPrices::market(10.0),
            TradingMode::Paper,
        )
    }
}

impl OrderPrices {
    pub const fn market(reference: f64) -> Self {
        Self {
            reference,
            limit: None,
            stop: None,
        }
    }
}

pub fn intent_from_pine_alert(
    script: &GeneratedPineScript,
    payload: PineAlertPayload,
) -> Result<OrderIntent, TradingError> {
    if script.variant != ScriptVariant::Strategy || script.source_strategy_id != payload.strategy_id
    {
        return Err(TradingError::PolicyDenied);
    }
    if script.symbol != payload.instrument {
        return Err(TradingError::PolicyDenied);
    }
    Ok(OrderIntent {
        strategy_id: payload.strategy_id,
        instrument: payload.instrument,
        side: payload.side,
        order_type: payload.order_type,
        quantity: payload.quantity,
        prices: payload.prices,
        tif: payload.tif,
        mode: payload.mode,
        governor_required: true,
        source: IntentSource::PineAlert {
            script_id: payload.script_id,
        },
    })
}

pub fn pre_trade_gate(
    governor: &RiskGovernor,
    intent: OrderIntent,
    account: &AccountState,
    market: &MarketState,
) -> Result<GatedOrderIntent, RiskGovernorError> {
    let decision = governor.decide(&intent, intent.mode, account, market)?;
    Ok(GatedOrderIntent { intent, decision })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pine_lab::{AlertHandoff, PineInput};
    use crate::risk_governor::RiskDecisionStatus;
    use crate::risk_policy::RiskPolicy;

    fn script() -> GeneratedPineScript {
        GeneratedPineScript {
            source_strategy_id: "strategy-a".into(),
            symbol: "SPY".into(),
            variant: ScriptVariant::Strategy,
            source: "//@version=6\nstrategy('x')".into(),
            alert_handoff: AlertHandoff::OrderIntent,
            inputs: Vec::<PineInput>::new(),
        }
    }

    fn payload() -> PineAlertPayload {
        PineAlertPayload {
            strategy_id: "strategy-a".into(),
            instrument: "SPY".into(),
            side: OrderSide::Buy,
            order_type: OrderType::Market,
            quantity: 1.0,
            prices: OrderPrices::market(10.0),
            tif: TimeInForce::Day,
            mode: TradingMode::Paper,
            script_id: "script-1".into(),
        }
    }

    #[test]
    fn ac_024_pine_alert_becomes_non_authoritative_gated_intent() {
        let intent = intent_from_pine_alert(&script(), payload()).unwrap();
        assert!(intent.governor_required);
        assert!(matches!(intent.source, IntentSource::PineAlert { .. }));
        assert_eq!(intent.mode, TradingMode::Paper);
    }

    #[test]
    fn ac_024_pine_alert_never_executes_or_bypasses_governor() {
        let governor = RiskGovernor::new(RiskPolicy::default());
        let mut intent = intent_from_pine_alert(&script(), payload()).unwrap();
        intent.governor_required = false;
        let gated = pre_trade_gate(
            &governor,
            intent,
            &AccountState::default(),
            &MarketState::default(),
        )
        .unwrap();
        assert_eq!(gated.decision.control_id, Some("REQ-RISK-017"));
        assert_ne!(gated.decision.status, RiskDecisionStatus::Approved);
    }

    #[test]
    fn ac_029_pre_trade_gate_approves_before_submit_path() {
        let governor = RiskGovernor::new(RiskPolicy::default());
        let gated = pre_trade_gate(
            &governor,
            OrderIntent::default(),
            &AccountState::default(),
            &MarketState::default(),
        )
        .unwrap();
        assert_eq!(gated.decision.status, RiskDecisionStatus::Approved);
    }

    #[test]
    fn paper_and_live_share_one_schema_and_mode_only_binds_adapter() {
        let mut paper = OrderIntent::default();
        let mut live = paper.clone();
        live.mode = TradingMode::LivePilot;
        paper.mode = TradingMode::Paper;
        assert_eq!(paper.strategy_id, live.strategy_id);
        assert_eq!(paper.instrument, live.instrument);
        assert_eq!(paper.order_type, live.order_type);
        assert_ne!(paper.mode, live.mode);
    }
}
