use super::*;
use crate::adapters::broker::{BrokerOrder, BrokerPosition, CapabilityManifest};
use crate::order_intent::{OrderPrices, OrderSide, OrderType};

struct FakeBroker {
    manifest: CapabilityManifest,
    response: Result<BrokerResponse, BrokerError>,
    submits: u32,
    health: BrokerHealth,
}

impl FakeBroker {
    fn ok() -> Self {
        Self {
            manifest: CapabilityManifest::default(),
            response: Ok(BrokerResponse {
                broker_order_id: "brk-1".into(),
                status: BrokerOrderStatus::Accepted,
                message: "accepted".into(),
                filled_quantity: 0.0,
            }),
            submits: 0,
            health: BrokerHealth {
                healthy: true,
                last_seen_seconds: 1,
                message: "ok".into(),
            },
        }
    }
}

impl BrokerAdapter for FakeBroker {
    fn name(&self) -> &str {
        "fake"
    }

    fn capability_manifest(&self) -> &CapabilityManifest {
        &self.manifest
    }

    fn account_state(&self) -> Result<AccountState, BrokerError> {
        Ok(AccountState::default())
    }

    fn positions(&self) -> Result<Vec<BrokerPosition>, BrokerError> {
        Ok(vec![])
    }

    fn open_orders(&self) -> Result<Vec<BrokerOrder>, BrokerError> {
        Ok(vec![])
    }

    fn submit(&mut self, _intent: &OrderIntent) -> Result<BrokerResponse, BrokerError> {
        self.submits += 1;
        self.response.clone()
    }

    fn cancel(&mut self, id: &str) -> Result<BrokerResponse, BrokerError> {
        Ok(BrokerResponse {
            broker_order_id: id.into(),
            status: BrokerOrderStatus::Cancelled,
            message: "cancelled".into(),
            filled_quantity: 0.0,
        })
    }

    fn replace(
        &mut self,
        id: &str,
        _replacement: &OrderIntent,
    ) -> Result<BrokerResponse, BrokerError> {
        Ok(BrokerResponse {
            broker_order_id: id.into(),
            status: BrokerOrderStatus::Accepted,
            message: "replaced".into(),
            filled_quantity: 0.0,
        })
    }

    fn fills(
        &self,
        _broker_order_id: &str,
    ) -> Result<Vec<crate::adapters::broker::BrokerFill>, BrokerError> {
        Ok(vec![])
    }

    fn health(&self) -> Result<BrokerHealth, BrokerError> {
        Ok(self.health.clone())
    }
}

fn live_intent(order_type: OrderType) -> OrderIntent {
    OrderIntent::new(
        "strategy-a",
        "SPY",
        OrderSide::Buy,
        order_type,
        1.0,
        OrderPrices::market(10.0),
        TradingMode::LivePilot,
    )
}

fn terminal(broker: FakeBroker) -> LiveTerminal<FakeBroker> {
    let policy = RiskPolicy::default();
    LiveTerminal::new(broker, RiskGovernor::new(policy.clone()), policy)
}

#[test]
fn t_term_01_live_and_paper_share_order_intent_interface() {
    let mut terminal = terminal(FakeBroker::ok());
    let gated = terminal
        .submit_order(
            live_intent(OrderType::Market),
            &AccountState::default(),
            &MarketState::default(),
        )
        .unwrap();
    assert_eq!(gated.intent.mode, TradingMode::LivePilot);
    assert_eq!(terminal.ledger()[0].status, OrderStatus::Requested);
    assert_eq!(terminal.ledger()[1].status, OrderStatus::Accepted);
}

#[test]
fn ec_trl_24_unsupported_order_type_fails_closed() {
    let mut broker = FakeBroker::ok();
    broker.manifest.stop_limit = false;
    let mut terminal = terminal(broker);
    let error = terminal
        .submit_order(
            live_intent(OrderType::StopLimit),
            &AccountState::default(),
            &MarketState::default(),
        )
        .unwrap_err();
    assert_eq!(error, LiveTerminalError::UnsupportedOrderType);
    assert!(terminal.ledger().is_empty());
}

#[test]
fn ec_trl_25_broker_reject_has_no_live_auto_retry() {
    let mut broker = FakeBroker::ok();
    broker.response = Err(BrokerError::Timeout("timeout".into()));
    let mut terminal = terminal(broker);
    let error = terminal
        .submit_order(
            live_intent(OrderType::Market),
            &AccountState::default(),
            &MarketState::default(),
        )
        .unwrap_err();
    assert_eq!(
        error,
        LiveTerminalError::BrokerReject("BROKER_REJECT".into())
    );
    assert_eq!(terminal.adapter().submits, 1);
    assert_eq!(
        terminal.ledger().last().unwrap().status,
        OrderStatus::Rejected
    );
}

#[test]
fn ec_trl_26_partial_reject_cancel_are_distinct_immutable_records() {
    let mut broker = FakeBroker::ok();
    broker.response = Ok(BrokerResponse {
        broker_order_id: "brk-1".into(),
        status: BrokerOrderStatus::Partial,
        message: "partial".into(),
        filled_quantity: 0.5,
    });
    let mut terminal = terminal(broker);
    let intent = live_intent(OrderType::Market);
    terminal
        .submit_order(
            intent.clone(),
            &AccountState::default(),
            &MarketState::default(),
        )
        .unwrap();
    terminal.cancel_order("brk-1", &intent).unwrap();
    let statuses: Vec<OrderStatus> = terminal.ledger().iter().map(|entry| entry.status).collect();
    assert_eq!(
        statuses,
        vec![
            OrderStatus::Requested,
            OrderStatus::Partial,
            OrderStatus::Cancelled
        ]
    );
    assert!(
        terminal
            .ledger()
            .iter()
            .all(|entry| !entry.immutable_hash.is_empty())
    );
}

#[test]
fn nfr_004_health_poll_halts_on_timeout_without_auto_resume() {
    let mut broker = FakeBroker::ok();
    broker.health = BrokerHealth {
        healthy: true,
        last_seen_seconds: 4,
        message: "stale".into(),
    };
    let terminal = terminal(broker);
    let decision = terminal.poll_health();
    assert!(decision.halt_required);
    assert!(!decision.auto_resume_allowed);
    assert_eq!(decision.poll_interval_ms, 1_000);
    assert_eq!(decision.halt_deadline_ms, 1_000);
}
