use super::*;
use crate::maker_checker::MakerCheckerApproval;
use serial_test::serial;

#[test]
#[serial]
fn upward_change_requires_maker_checker_and_audit_log() {
    let current = RiskPolicy::default();
    let mut next = current.clone();
    next.thresholds.max_daily_loss_pct = 3.0;
    let blocked = current
        .apply_change(next.clone(), None, None, "risk-admin")
        .unwrap_err();
    assert_eq!(blocked, RiskPolicyError::UpwardChangeNeedsMakerChecker);

    let dir = tempfile::tempdir().unwrap();
    let ledger = AuditLedger::open(dir.path().join("audit.jsonl")).unwrap();
    let approval =
        MakerCheckerApproval::new("rp-1", "alice", "bob", "raise-risk-limit", true, "approved");
    let updated = current
        .apply_change(next, Some(&approval), Some(&ledger), "risk-admin")
        .unwrap();
    assert!(updated.validate_hash());
    assert_eq!(ledger.records().unwrap().len(), 1);
}

#[test]
fn downward_change_is_free_and_content_addressed() {
    let current = RiskPolicy::default();
    let mut next = current.clone();
    next.thresholds.max_order_notional_pct = 1.0;
    let updated = current
        .apply_change(next, None, None, "risk-admin")
        .unwrap();
    assert!(updated.validate_hash());
    assert_ne!(updated.version_hash, current.version_hash);
}

#[test]
#[serial]
fn state_survives_restart_and_restores_auto_halt() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("risk-state.db");
    let state = RiskRuntimeState {
        strategy_id: "strat-a".to_string(),
        consecutive_losses: 2,
        daily_loss_cents: 1234,
        cooldown_until_unix_ms: Some(99),
        restart_auto_halt: false,
    };
    RiskStateStore::open(&db_path)
        .unwrap()
        .persist_state(&state)
        .unwrap();
    let restored = RiskStateStore::open(&db_path)
        .unwrap()
        .restore_state("strat-a")
        .unwrap()
        .unwrap();
    assert_eq!(restored.consecutive_losses, 2);
    assert!(restored.restart_auto_halt);
}
