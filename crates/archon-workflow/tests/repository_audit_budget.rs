use archon_workflow::repository_audit::budget::{AuditBudget, AuditPolicy, Limit};

#[test]
fn finite_allowance_above_hour_and_unlimited_have_no_hidden_cutoff() {
    let policy = AuditPolicy {
        attempt_timeout_secs: Limit::Finite(7200),
        total_time_secs: Limit::Unlimited,
        unexpected_change_refreshes: Limit::Finite(12),
    };
    let mut budget = AuditBudget::new(policy);
    assert_eq!(budget.begin("one", 0, false).unwrap(), Some(7_200_000));
    budget.finish("one", 4_000_000).unwrap();
    assert_eq!(
        budget.begin("two", 4_000_000, false).unwrap(),
        Some(7_200_000)
    );
    assert_eq!(budget.spent_ms, 4_000_000);
}
#[test]
fn configured_count_and_time_pause_without_reset_on_serialization() {
    let mut budget = AuditBudget::new(AuditPolicy {
        attempt_timeout_secs: Limit::Finite(10),
        total_time_secs: Limit::Finite(15),
        unexpected_change_refreshes: Limit::Finite(1),
    });
    budget.begin("one", 0, true).unwrap();
    budget.finish("one", 7000).unwrap();
    let mut budget: AuditBudget =
        serde_json::from_value(serde_json::to_value(&budget).unwrap()).unwrap();
    assert!(
        budget
            .begin("two", 7000, true)
            .unwrap_err()
            .to_string()
            .contains("paused")
    );
    assert_eq!(budget.begin("two", 7000, false).unwrap(), Some(8000));
    budget.finish("two", 15000).unwrap();
    assert!(budget.begin("three", 15000, false).is_err());
    assert_eq!(budget.spent_ms, 15000);
}
#[test]
fn simultaneous_attempts_cannot_each_spend_remaining_budget() {
    let mut budget = AuditBudget::new(AuditPolicy {
        attempt_timeout_secs: Limit::Finite(10),
        total_time_secs: Limit::Finite(15),
        unexpected_change_refreshes: Limit::Unlimited,
    });
    budget.begin("one", 100, false).unwrap();
    assert!(budget.begin("two", 100, false).is_err());
    budget.recover_interrupted(4100).unwrap();
    assert_eq!(budget.spent_ms, 4000);
    assert_eq!(budget.begin("two", 4100, false).unwrap(), Some(10000));
}
#[test]
fn an_interrupted_unlimited_attempt_does_not_restore_usage() {
    let mut budget = AuditBudget::new(AuditPolicy {
        attempt_timeout_secs: Limit::Unlimited,
        total_time_secs: Limit::Unlimited,
        unexpected_change_refreshes: Limit::Unlimited,
    });
    assert_eq!(budget.begin("one", 100, true).unwrap(), None);
    budget.heartbeat("one", 6100).unwrap();
    let mut loaded: AuditBudget =
        serde_json::from_value(serde_json::to_value(&budget).unwrap()).unwrap();
    loaded.recover_interrupted(7100).unwrap();
    assert_eq!(loaded.spent_ms, 7000);
    assert_eq!(loaded.unexpected_refreshes, 1);
}
