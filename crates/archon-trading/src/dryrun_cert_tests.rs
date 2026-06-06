use super::*;
use crate::adapters::broker::{
    BrokerError, BrokerFill, BrokerHealth, BrokerOrder, BrokerOrderStatus, BrokerResponse,
    CapabilityManifest,
};
use crate::kill_switch::CancelReport;
use crate::order_intent::{OrderPrices, OrderSide, OrderType, TradingMode};

#[derive(Clone)]
struct DryRunBroker {
    manifest: CapabilityManifest,
    health: BrokerHealth,
}

impl DryRunBroker {
    fn passing() -> Self {
        Self {
            manifest: CapabilityManifest::default(),
            health: BrokerHealth {
                healthy: true,
                last_seen_seconds: 0,
                message: "dry-run healthy".into(),
            },
        }
    }
}

impl BrokerAdapter for DryRunBroker {
    fn name(&self) -> &str {
        "dry-run"
    }

    fn capability_manifest(&self) -> &CapabilityManifest {
        &self.manifest
    }

    fn account_state(&self) -> Result<AccountState, BrokerError> {
        Ok(AccountState::default())
    }

    fn positions(&self) -> Result<Vec<crate::adapters::broker::BrokerPosition>, BrokerError> {
        Ok(vec![])
    }

    fn open_orders(&self) -> Result<Vec<BrokerOrder>, BrokerError> {
        Ok(vec![])
    }

    fn submit(&mut self, intent: &OrderIntent) -> Result<BrokerResponse, BrokerError> {
        response(
            "dry-submit",
            intent.instrument.clone(),
            BrokerOrderStatus::Accepted,
        )
    }

    fn cancel(&mut self, broker_order_id: &str) -> Result<BrokerResponse, BrokerError> {
        response(broker_order_id, "SPY".into(), BrokerOrderStatus::Cancelled)
    }

    fn replace(
        &mut self,
        broker_order_id: &str,
        _replacement: &OrderIntent,
    ) -> Result<BrokerResponse, BrokerError> {
        response(broker_order_id, "SPY".into(), BrokerOrderStatus::Accepted)
    }

    fn fills(&self, _broker_order_id: &str) -> Result<Vec<BrokerFill>, BrokerError> {
        Ok(vec![])
    }

    fn health(&self) -> Result<BrokerHealth, BrokerError> {
        Ok(self.health.clone())
    }
}

fn response(
    id: &str,
    instrument: String,
    status: BrokerOrderStatus,
) -> Result<BrokerResponse, BrokerError> {
    Ok(BrokerResponse {
        broker_order_id: format!("{id}-{instrument}"),
        status,
        message: "dry-run".into(),
        filled_quantity: 0.0,
    })
}

fn live_intent() -> OrderIntent {
    OrderIntent::new(
        "strategy-a",
        "SPY",
        OrderSide::Buy,
        OrderType::Market,
        1.0,
        OrderPrices::market(10.0),
        TradingMode::LivePilot,
    )
}

fn passing_report() -> CertificationReport {
    let kill_switch = KillSwitch::new(|| {
        Ok(CancelReport {
            requested: 2,
            cancelled: 2,
        })
    });
    certify_live_adapter(
        DryRunBroker::passing(),
        live_intent(),
        AccountState::default(),
        MarketState::default(),
        RiskPolicy::default(),
        &kill_switch,
    )
}

#[test]
fn t_term_03_certification_must_pass_every_check_before_enablement() {
    let report = passing_report();
    assert!(report.passed, "failed checks: {:?}", report.failed_checks());
    assert!(can_enable_live(&report));
}

#[test]
fn a_live_01_ninety_nine_percent_certification_still_blocks_enablement() {
    let mut report = passing_report();
    report.checks.push(CertificationCheck {
        id: "forced-one-percent-gap".to_string(),
        passed: false,
    });
    report.passed = report.checks.iter().all(|check| check.passed);
    assert!(!can_enable_live(&report));
}

#[test]
fn nfr_001_and_nfr_002_are_explicit_certification_gates() {
    let report = passing_report();
    assert!(report.pre_trade_p99_ms <= PRE_TRADE_P99_SLO_MS);
    assert!(
        report
            .checks
            .iter()
            .any(|check| check.id == "NFR-002:in-app" && check.passed)
    );
    assert!(
        report
            .checks
            .iter()
            .any(|check| check.id == "NFR-002:out-of-band" && check.passed)
    );
}
