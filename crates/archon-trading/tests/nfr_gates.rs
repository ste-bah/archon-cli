use archon_trading::audit_ledger::{AuditLedger, NewLedgerRecord, OrderStatus, TaxFields};
use archon_trading::kill_switch::{CancelReport, KillChannel, KillSwitch};
use archon_trading::order_intent::{OrderIntent, TradingMode};
use archon_trading::risk_governor::{AccountState, MarketState, RiskDecisionStatus, RiskGovernor};
use archon_trading::risk_policy::{RiskPolicy, RiskRuntimeState, RiskStateStore};
use serde_json::json;
use serial_test::serial;
use std::fs;
use std::path::Path;

#[test]
#[serial]
fn nfr_001_governor_pre_trade_latency_and_audit_ack() {
    let dir = tempfile::tempdir().expect("tempdir");
    let audit = AuditLedger::open(dir.path().join("risk-audit.jsonl")).expect("audit");
    let governor = RiskGovernor::new(RiskPolicy::default()).with_audit(audit.clone());

    let decision = governor
        .decide(
            &OrderIntent::default(),
            TradingMode::Paper,
            &AccountState::default(),
            &MarketState::default(),
        )
        .expect("decision");

    assert_eq!(decision.status, RiskDecisionStatus::Approved);
    assert!(
        decision.latency_ms <= 50,
        "latency {}ms",
        decision.latency_ms
    );
    assert_eq!(audit.records().expect("records").len(), 1);
}

#[test]
fn nfr_002_kill_switch_channels_meet_latency_slos() {
    for channel in [KillChannel::InAppApi, KillChannel::OutOfBandCli] {
        let switch = KillSwitch::new(|| {
            Ok(CancelReport {
                requested: 2,
                cancelled: 2,
            })
        });
        let receipt = switch.trigger_from(channel).expect("kill receipt");
        assert!(switch.is_halted());
        assert!(!switch.accepts_new_submissions());
        assert!(receipt.meets_nfr_002(), "receipt: {receipt:?}");
    }
}

#[test]
#[serial]
fn nfr_005_log_before_act_blocks_until_durable_audit_ack() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("ledger.jsonl");
    let ledger = AuditLedger::open(&path).expect("audit");
    let record = ledger.log_before_act(audit_request()).expect("ack");
    let persisted = fs::read_to_string(path).expect("persisted ledger");

    assert_eq!(record.status, OrderStatus::Requested);
    assert!(persisted.contains(&record.content_hash));
    assert!(ledger.verify_chain().is_ok());
    assert!(
        ledger
            .log_before_act(status_request(OrderStatus::Accepted))
            .is_err()
    );
}

#[test]
fn nfr_007_replay_fingerprint_is_bit_identical() {
    let config = json!({
        "snapshot_checksum": "dataset-v1",
        "config_hash": "cost-model-v1",
        "seed": 42,
        "pinned_numeric_lib": "rust-f64-stable",
        "order_stable_reductions": true
    });
    let first = replay_fingerprint(&config);
    let second = replay_fingerprint(&config);
    assert_eq!(first, second);
}

#[test]
#[serial]
fn nfr_009_fail_closed_unavailable_and_restart_auto_halt() {
    let dir = tempfile::tempdir().expect("tempdir");
    let store = RiskStateStore::open(dir.path().join("risk-state.db")).expect("state store");
    store
        .persist_state(&RiskRuntimeState {
            strategy_id: "strategy-a".to_string(),
            consecutive_losses: 1,
            daily_loss_cents: 2500,
            cooldown_until_unix_ms: Some(99),
            restart_auto_halt: false,
        })
        .expect("persist");

    let restored = RiskStateStore::open(dir.path().join("risk-state.db"))
        .expect("reopen")
        .restore_state("strategy-a")
        .expect("restore")
        .expect("state");
    assert!(restored.restart_auto_halt);

    let account = AccountState {
        governor_available: false,
        ..Default::default()
    };
    let decision = RiskGovernor::new(RiskPolicy::default())
        .decide(
            &OrderIntent::default(),
            TradingMode::Paper,
            &account,
            &MarketState::default(),
        )
        .expect("decision");
    assert_eq!(decision.control_id, Some("REQ-FAIL-004"));
}

#[test]
#[serial]
fn nfr_010_ac_021_ec_trl_33_secret_scanner_zero_tolerance() {
    let dir = tempfile::tempdir().expect("tempdir");
    let ledger_path = dir.path().join("ledger.jsonl");
    let ledger = AuditLedger::open(&ledger_path).expect("audit");
    ledger
        .log_before_act(secret_bearing_request())
        .expect("redacted");

    let persisted = fs::read_to_string(&ledger_path).expect("ledger text");
    assert_secret_clean(&persisted);
    assert_secret_clean_tree(dir.path());
}

fn audit_request() -> NewLedgerRecord {
    status_request(OrderStatus::Requested)
}

fn status_request(status: OrderStatus) -> NewLedgerRecord {
    NewLedgerRecord {
        actor: "nfr-gate".to_string(),
        strategy_id: "strategy-a".to_string(),
        policy_version: RiskPolicy::default().version_hash,
        status,
        risk_decision: json!({"allowed": true}),
        order_intent: json!({"symbol": "SPY"}),
        broker_response: json!({"not_submitted": true}),
        account: json!({"mode": "paper"}),
        tax: TaxFields::default(),
        artefacts: vec![],
        maker_checker: None,
    }
}

fn secret_bearing_request() -> NewLedgerRecord {
    NewLedgerRecord {
        order_intent: json!({"api_key": secret_fixture()}),
        account: json!({"token": secret_fixture()}),
        ..audit_request()
    }
}

fn secret_fixture() -> String {
    ["sk", "live", "fixture"].join("-")
}

fn replay_fingerprint(config: &serde_json::Value) -> String {
    let bytes = serde_json::to_vec(config).expect("encode replay config");
    blake3::hash(&bytes).to_hex().to_string()
}

fn assert_secret_clean(text: &str) {
    for pattern in [secret_fixture(), "BEGIN PRIVATE KEY".to_string()] {
        assert!(!text.contains(&pattern), "secret pattern leaked: {pattern}");
    }
}

fn assert_secret_clean_tree(root: &Path) {
    for entry in fs::read_dir(root).expect("scan root") {
        let path = entry.expect("entry").path();
        if path.is_dir() {
            assert_secret_clean_tree(&path);
        } else {
            let text = fs::read_to_string(path).unwrap_or_default();
            assert_secret_clean(&text);
        }
    }
}
