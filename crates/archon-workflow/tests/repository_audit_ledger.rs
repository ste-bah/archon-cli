use archon_workflow::repository_audit::{AuditContract, AuditReport};
use archon_workflow::repository_audit::ledger::AuditLedger;
use serde_json::json;

fn report(snapshot: &str, verdict: &str) -> AuditReport {
    serde_json::from_value(json!({"schema_version":1,"snapshot":snapshot,"records":[{
        "declared_path":"new.txt","verdict":verdict,
        "equivalents":if verdict=="exists_elsewhere"{vec!["old.txt"]}else{vec![]},
        "required_action":if verdict=="exists_elsewhere"{"wire_or_migrate"}else{"none"},"reason":"inspected behavior"
    }]})).unwrap()
}
#[test]
fn explanation_cannot_close_obligation_and_wrong_snapshot_cannot_pass() {
    let mut ledger = AuditLedger::default();
    ledger.accept(AuditContract{schema_version:1,snapshot:"one".into(),declared_paths:vec!["new.txt".into()]},report("one","exists_elsewhere")).unwrap();
    assert!(!ledger.unresolved("one").unwrap().is_empty());
    assert!(ledger.unresolved("two").is_err());
    let mut copied=ledger.clone();
    copied.propose("new.txt", "fluent explanation".into());
    assert!(!copied.unresolved("one").unwrap().is_empty());
}
#[test]
fn refreshed_assessment_retains_original_judgment_and_needs_applied_evidence() {
    let mut ledger=AuditLedger::default();
    let contract=|s:&str|AuditContract{schema_version:1,snapshot:s.into(),declared_paths:vec!["new.txt".into()]};
    ledger.accept(contract("one"),report("one","exists_elsewhere")).unwrap();
    ledger.accept(contract("two"),report("two","exists_as_declared")).unwrap();
    assert!(!ledger.unresolved("two").unwrap().is_empty(),"positive text is not successful apply evidence");
    ledger.record_applied("new.txt", "commit".into());
    ledger.accept(contract("three"),report("three","exists_as_declared")).unwrap();
    assert!(ledger.unresolved("three").unwrap().is_empty());
    assert_eq!(ledger.history.len(),3);
    assert_eq!(ledger.history[0].records[0].equivalents,vec!["old.txt"]);
}
