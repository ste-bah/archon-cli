use crate::adapters::broker::BrokerAdapter;
use crate::kill_switch::{KillChannel, KillReceipt, KillSwitch};
use crate::live_terminal::LiveTerminal;
use crate::order_intent::OrderIntent;
use crate::risk_governor::{AccountState, MarketState, RiskGovernor};
use crate::risk_policy::RiskPolicy;
use serde::{Deserialize, Serialize};
use std::time::{Duration, Instant};

pub const PRE_TRADE_P99_SLO_MS: u128 = 50;
const LATENCY_SAMPLES: usize = 20;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CertificationReport {
    pub adapter_name: String,
    pub passed: bool,
    pub checks: Vec<CertificationCheck>,
    pub pre_trade_p99_ms: u128,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CertificationCheck {
    pub id: String,
    pub passed: bool,
}

pub fn certify_live_adapter<A: BrokerAdapter>(
    adapter: A,
    sample_intent: OrderIntent,
    account: AccountState,
    market: MarketState,
    policy: RiskPolicy,
    kill_switch: &KillSwitch,
) -> CertificationReport {
    let adapter_name = adapter.name().to_string();
    let mut suite = CertificationSuite::new(adapter, sample_intent, account, market, policy);
    let mut checks = Vec::new();
    checks.push(suite.check_manifest());
    checks.push(suite.check_submit_cancel_replace_and_ledger());
    checks.push(suite.check_health());
    let latency = suite.check_pre_trade_latency();
    let p99 = suite.pre_trade_p99_ms;
    checks.push(latency);
    checks.extend(check_kill_switch_channels(kill_switch));
    let passed = checks.iter().all(|check| check.passed);
    CertificationReport {
        adapter_name,
        passed,
        checks,
        pre_trade_p99_ms: p99,
    }
}

pub fn can_enable_live(report: &CertificationReport) -> bool {
    report.passed && report.checks.iter().all(|check| check.passed)
}

struct CertificationSuite<A: BrokerAdapter> {
    terminal: LiveTerminal<A>,
    intent: OrderIntent,
    account: AccountState,
    market: MarketState,
    pre_trade_p99_ms: u128,
}

impl<A: BrokerAdapter> CertificationSuite<A> {
    fn new(
        adapter: A,
        intent: OrderIntent,
        account: AccountState,
        market: MarketState,
        policy: RiskPolicy,
    ) -> Self {
        let governor = RiskGovernor::new(policy.clone());
        Self {
            terminal: LiveTerminal::new(adapter, governor, policy),
            intent,
            account,
            market,
            pre_trade_p99_ms: u128::MAX,
        }
    }

    fn check_manifest(&self) -> CertificationCheck {
        let result = self
            .terminal
            .adapter()
            .capability_manifest()
            .require_supported(&self.intent);
        check(
            "REQ-TERM-006:manifest",
            result.is_ok(),
            "manifest supports dry-run intent",
        )
    }

    fn check_submit_cancel_replace_and_ledger(&mut self) -> CertificationCheck {
        let result = self.exercise_order_path();
        check(
            "REQ-TERM-006:submit-cancel-replace-ledger",
            result.is_ok(),
            result
                .err()
                .unwrap_or_else(|| "dry-run order path passed".to_string()),
        )
    }

    fn exercise_order_path(&mut self) -> Result<(), String> {
        self.terminal
            .submit_order(self.intent.clone(), &self.account, &self.market)
            .map_err(|err| format!("submit failed: {err:?}"))?;
        let order_id = latest_order_id(&self.terminal).ok_or("missing broker order id")?;
        self.terminal
            .cancel_order(&order_id, &self.intent)
            .map_err(|err| format!("cancel failed: {err:?}"))?;
        self.terminal
            .replace_order(&order_id, &self.intent)
            .map_err(|err| format!("replace failed: {err:?}"))?;
        (self.terminal.ledger().len() >= 4)
            .then_some(())
            .ok_or_else(|| "ledger missing distinct status records".to_string())
    }

    fn check_health(&self) -> CertificationCheck {
        let decision = self.terminal.poll_health();
        check(
            "REQ-TERM-006:health",
            decision.healthy && !decision.halt_required,
            decision.reason,
        )
    }

    fn check_pre_trade_latency(&mut self) -> CertificationCheck {
        let mut samples = Vec::with_capacity(LATENCY_SAMPLES);
        for _ in 0..LATENCY_SAMPLES {
            let started = Instant::now();
            let result =
                self.terminal
                    .submit_order(self.intent.clone(), &self.account, &self.market);
            samples.push(started.elapsed());
            if result.is_err() {
                break;
            }
        }
        self.pre_trade_p99_ms = p99_ms(&mut samples);
        check(
            "NFR-001:pre-trade-p99",
            self.pre_trade_p99_ms <= PRE_TRADE_P99_SLO_MS,
            format!("p99={}ms", self.pre_trade_p99_ms),
        )
    }
}

fn check_kill_switch_channels(kill_switch: &KillSwitch) -> Vec<CertificationCheck> {
    vec![
        kill_channel_check(
            kill_switch.trigger_from(KillChannel::InAppApi),
            "NFR-002:in-app",
        ),
        kill_channel_check(
            kill_switch.trigger_from(KillChannel::OutOfBandCli),
            "NFR-002:out-of-band",
        ),
    ]
}

fn kill_channel_check(
    result: Result<KillReceipt, crate::kill_switch::KillSwitchError>,
    id: &'static str,
) -> CertificationCheck {
    match result {
        Ok(receipt) => check(
            id,
            receipt.meets_nfr_002(),
            format!(
                "halt={}ms cancel={}ms",
                receipt.halt_latency_ms, receipt.cancel_latency_ms
            ),
        ),
        Err(err) => check(id, false, err.to_string()),
    }
}

fn latest_order_id<A: BrokerAdapter>(terminal: &LiveTerminal<A>) -> Option<String> {
    terminal
        .ledger()
        .iter()
        .rev()
        .find_map(|entry| entry.broker_order_id.clone())
}

fn p99_ms(samples: &mut [Duration]) -> u128 {
    if samples.is_empty() {
        return u128::MAX;
    }
    samples.sort_unstable();
    let index = ((samples.len() - 1) * 99) / 100;
    samples[index].as_millis()
}

fn check(id: &'static str, passed: bool, _detail: impl Into<String>) -> CertificationCheck {
    CertificationCheck {
        id: id.to_string(),
        passed,
    }
}

impl CertificationReport {
    pub fn failed_checks(&self) -> Vec<&CertificationCheck> {
        self.checks.iter().filter(|check| !check.passed).collect()
    }
}

#[cfg(test)]
mod tests {
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
}
