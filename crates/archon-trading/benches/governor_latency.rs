use archon_trading::audit_ledger::{AuditLedger, NewLedgerRecord, OrderStatus, TaxFields};
use archon_trading::order_intent::{OrderIntent, TradingMode};
use archon_trading::risk_governor::{AccountState, MarketState, RiskGovernor};
use archon_trading::risk_policy::RiskPolicy;
use serde_json::json;
use std::time::{Duration, Instant};

const P50_SLO: Duration = Duration::from_millis(20);
const P99_SLO: Duration = Duration::from_millis(50);

#[test]
fn governor_latency_includes_durable_audit_write() {
    let dir = tempfile::tempdir().expect("tempdir");
    let audit = AuditLedger::open(dir.path().join("governor-latency.jsonl")).expect("audit");
    let governor = RiskGovernor::new(RiskPolicy::default()).with_audit(audit.clone());
    let intent = OrderIntent::default();
    let account = AccountState::default();
    let market = MarketState::default();
    let mut samples = Vec::new();

    for _ in 0..32 {
        let started = Instant::now();
        governor
            .decide(&intent, TradingMode::Paper, &account, &market)
            .expect("risk decision");
        samples.push(started.elapsed());
    }

    samples.sort_unstable();
    let p50 = percentile(&samples, 50);
    let p99 = percentile(&samples, 99);
    assert!(p50 <= P50_SLO, "p50 {p50:?} exceeds {P50_SLO:?}");
    assert!(p99 <= P99_SLO, "p99 {p99:?} exceeds {P99_SLO:?}");
    assert_eq!(audit.records().expect("audit records").len(), samples.len());
}

#[test]
fn log_before_act_bench_fixture_compiles_with_fsync_path() {
    let dir = tempfile::tempdir().expect("tempdir");
    let ledger = AuditLedger::open(dir.path().join("log-before-act.jsonl")).expect("audit");
    let started = Instant::now();
    ledger
        .log_before_act(NewLedgerRecord {
            actor: "nfr-bench".to_string(),
            strategy_id: "strategy-a".to_string(),
            policy_version: RiskPolicy::default().version_hash,
            status: OrderStatus::Requested,
            risk_decision: json!({"allowed": true}),
            order_intent: json!({"symbol": "SPY"}),
            broker_response: json!({"not_submitted": true}),
            account: json!({"mode": "paper"}),
            tax: TaxFields::default(),
            artefacts: vec![],
            maker_checker: None,
        })
        .expect("durable audit ack");
    assert!(started.elapsed() <= P99_SLO);
}

fn percentile(samples: &[Duration], percentile: usize) -> Duration {
    let index = ((samples.len().saturating_sub(1)) * percentile) / 100;
    samples[index]
}
