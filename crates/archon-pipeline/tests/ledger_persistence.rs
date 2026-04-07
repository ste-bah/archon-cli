use archon_pipeline::ledgers::{
    DecisionEntry, DecisionLedger, TaskEntry, TaskLedger, TaskStatus, VerificationEntry,
    VerificationLedger, VerificationRef, WiringObligationRef,
};
use tempfile::TempDir;

// ---------------------------------------------------------------------------
// Helper factories
// ---------------------------------------------------------------------------

fn make_decision_entry(id: &str) -> DecisionEntry {
    DecisionEntry {
        id: id.into(),
        decision: format!("Decision {id}"),
        reason: "Testing".into(),
        source: "test-agent".into(),
        affected: vec!["src/lib.rs".into()],
        timestamp: "2026-04-07T12:00:00Z".into(),
    }
}

fn make_task_entry(task_id: &str) -> TaskEntry {
    TaskEntry {
        task_id: task_id.into(),
        status: TaskStatus::Pending,
        assigned_agent: "coder".into(),
        dependencies: vec!["dep-1".into()],
        changed_files: vec!["src/main.rs".into()],
        wiring_obligations: vec![WiringObligationRef {
            obligation_id: "WO-001".into(),
            status: "pending".into(),
        }],
        last_verification: Some(VerificationRef {
            gate_name: "gate-1".into(),
            passed: true,
            timestamp: "2026-04-07T12:00:00Z".into(),
        }),
        timestamp: "2026-04-07T12:00:00Z".into(),
    }
}

fn make_verification_entry(gate: &str, passed: bool) -> VerificationEntry {
    VerificationEntry {
        gate_name: gate.into(),
        passed,
        failure_details: if passed {
            None
        } else {
            Some("something failed".into())
        },
        evidence_summary: format!("Evidence for {gate}"),
        timestamp: "2026-04-07T12:00:00Z".into(),
    }
}

// ===========================================================================
// DecisionLedger tests
// ===========================================================================

#[test]
fn decision_ledger_creates_file_at_correct_path() {
    let dir = TempDir::new().unwrap();
    let ledger = DecisionLedger::new(dir.path());
    assert!(ledger.path().ends_with("ledgers/decisions.json"));
    ledger.append(&make_decision_entry("D-001")).unwrap();
    assert!(ledger.path().exists());
}

#[test]
fn decision_ledger_append_and_load() {
    let dir = TempDir::new().unwrap();
    let ledger = DecisionLedger::new(dir.path());
    let entry = make_decision_entry("D-001");
    ledger.append(&entry).unwrap();
    let entries = ledger.load_all().unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].id, "D-001");
}

#[test]
fn decision_ledger_append_preserves_existing() {
    let dir = TempDir::new().unwrap();
    let ledger = DecisionLedger::new(dir.path());
    ledger.append(&make_decision_entry("D-001")).unwrap();
    ledger.append(&make_decision_entry("D-002")).unwrap();
    let entries = ledger.load_all().unwrap();
    assert_eq!(entries.len(), 2);
    assert_eq!(entries[0].id, "D-001");
    assert_eq!(entries[1].id, "D-002");
}

#[test]
fn decision_ledger_reload_after_append() {
    let dir = TempDir::new().unwrap();
    let ledger = DecisionLedger::new(dir.path());
    ledger.append(&make_decision_entry("D-001")).unwrap();
    ledger.append(&make_decision_entry("D-002")).unwrap();
    // Simulate "crash" by creating a fresh ledger instance pointing at same dir
    let ledger2 = DecisionLedger::new(dir.path());
    let entries = ledger2.load_all().unwrap();
    assert_eq!(entries.len(), 2);
    assert_eq!(entries[0].id, "D-001");
    assert_eq!(entries[1].id, "D-002");
}

#[test]
fn decision_ledger_empty_load() {
    let dir = TempDir::new().unwrap();
    let ledger = DecisionLedger::new(dir.path());
    let entries = ledger.load_all().unwrap();
    assert!(entries.is_empty());
}

#[test]
fn decision_ledger_serialization_round_trip() {
    let dir = TempDir::new().unwrap();
    let ledger = DecisionLedger::new(dir.path());
    let entry = DecisionEntry {
        id: "D-RT".into(),
        decision: "Round-trip test".into(),
        reason: "Verify serialization".into(),
        source: "tester".into(),
        affected: vec!["a.rs".into(), "b.rs".into()],
        timestamp: "2026-04-07T13:00:00Z".into(),
    };
    ledger.append(&entry).unwrap();
    let loaded = ledger.load_all().unwrap();
    assert_eq!(loaded[0].id, entry.id);
    assert_eq!(loaded[0].decision, entry.decision);
    assert_eq!(loaded[0].reason, entry.reason);
    assert_eq!(loaded[0].source, entry.source);
    assert_eq!(loaded[0].affected, entry.affected);
    assert_eq!(loaded[0].timestamp, entry.timestamp);
}

#[test]
fn decision_ledger_chronological_order() {
    let dir = TempDir::new().unwrap();
    let ledger = DecisionLedger::new(dir.path());
    for i in 0..5 {
        ledger
            .append(&make_decision_entry(&format!("D-{i:03}")))
            .unwrap();
    }
    let entries = ledger.load_all().unwrap();
    let ids: Vec<&str> = entries.iter().map(|e| e.id.as_str()).collect();
    assert_eq!(ids, vec!["D-000", "D-001", "D-002", "D-003", "D-004"]);
}

// ===========================================================================
// TaskLedger tests
// ===========================================================================

#[test]
fn task_ledger_creates_file_at_correct_path() {
    let dir = TempDir::new().unwrap();
    let ledger = TaskLedger::new(dir.path());
    assert!(ledger.path().ends_with("ledgers/tasks.json"));
    ledger.append(&make_task_entry("T-001")).unwrap();
    assert!(ledger.path().exists());
}

#[test]
fn task_ledger_append_and_load() {
    let dir = TempDir::new().unwrap();
    let ledger = TaskLedger::new(dir.path());
    ledger.append(&make_task_entry("T-001")).unwrap();
    let entries = ledger.load_all().unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].task_id, "T-001");
    assert_eq!(entries[0].status, TaskStatus::Pending);
}

#[test]
fn task_ledger_multiple_appends_accumulate() {
    let dir = TempDir::new().unwrap();
    let ledger = TaskLedger::new(dir.path());
    for i in 1..=4 {
        ledger
            .append(&make_task_entry(&format!("T-{i:03}")))
            .unwrap();
    }
    let entries = ledger.load_all().unwrap();
    assert_eq!(entries.len(), 4);
}

#[test]
fn task_ledger_reload_after_append() {
    let dir = TempDir::new().unwrap();
    let ledger = TaskLedger::new(dir.path());
    ledger.append(&make_task_entry("T-001")).unwrap();
    ledger.append(&make_task_entry("T-002")).unwrap();
    let ledger2 = TaskLedger::new(dir.path());
    let entries = ledger2.load_all().unwrap();
    assert_eq!(entries.len(), 2);
    assert_eq!(entries[1].task_id, "T-002");
}

#[test]
fn task_ledger_empty_load() {
    let dir = TempDir::new().unwrap();
    let ledger = TaskLedger::new(dir.path());
    let entries = ledger.load_all().unwrap();
    assert!(entries.is_empty());
}

#[test]
fn task_ledger_serialization_round_trip() {
    let dir = TempDir::new().unwrap();
    let ledger = TaskLedger::new(dir.path());
    let entry = make_task_entry("T-RT");
    ledger.append(&entry).unwrap();
    let loaded = ledger.load_all().unwrap();
    assert_eq!(loaded[0].task_id, entry.task_id);
    assert_eq!(loaded[0].status, entry.status);
    assert_eq!(loaded[0].assigned_agent, entry.assigned_agent);
    assert_eq!(loaded[0].dependencies, entry.dependencies);
    assert_eq!(loaded[0].changed_files, entry.changed_files);
    assert_eq!(
        loaded[0].wiring_obligations[0].obligation_id,
        entry.wiring_obligations[0].obligation_id
    );
    let lv = loaded[0].last_verification.as_ref().unwrap();
    let ev = entry.last_verification.as_ref().unwrap();
    assert_eq!(lv.gate_name, ev.gate_name);
    assert_eq!(lv.passed, ev.passed);
}

// ===========================================================================
// VerificationLedger tests
// ===========================================================================

#[test]
fn verification_ledger_creates_file_at_correct_path() {
    let dir = TempDir::new().unwrap();
    let ledger = VerificationLedger::new(dir.path());
    assert!(ledger.path().ends_with("ledgers/verifications.json"));
    ledger
        .append(&make_verification_entry("gate-1", true))
        .unwrap();
    assert!(ledger.path().exists());
}

#[test]
fn verification_ledger_append_and_load() {
    let dir = TempDir::new().unwrap();
    let ledger = VerificationLedger::new(dir.path());
    ledger
        .append(&make_verification_entry("gate-1", true))
        .unwrap();
    let entries = ledger.load_all().unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].gate_name, "gate-1");
    assert!(entries[0].passed);
}

#[test]
fn verification_ledger_multiple_appends_accumulate() {
    let dir = TempDir::new().unwrap();
    let ledger = VerificationLedger::new(dir.path());
    ledger
        .append(&make_verification_entry("gate-1", true))
        .unwrap();
    ledger
        .append(&make_verification_entry("gate-2", false))
        .unwrap();
    ledger
        .append(&make_verification_entry("gate-3", true))
        .unwrap();
    let entries = ledger.load_all().unwrap();
    assert_eq!(entries.len(), 3);
    assert!(!entries[1].passed);
    assert!(entries[1].failure_details.is_some());
}

#[test]
fn verification_ledger_reload_after_append() {
    let dir = TempDir::new().unwrap();
    let ledger = VerificationLedger::new(dir.path());
    ledger
        .append(&make_verification_entry("gate-1", true))
        .unwrap();
    let ledger2 = VerificationLedger::new(dir.path());
    let entries = ledger2.load_all().unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].gate_name, "gate-1");
}

#[test]
fn verification_ledger_empty_load() {
    let dir = TempDir::new().unwrap();
    let ledger = VerificationLedger::new(dir.path());
    let entries = ledger.load_all().unwrap();
    assert!(entries.is_empty());
}

#[test]
fn verification_ledger_serialization_round_trip() {
    let dir = TempDir::new().unwrap();
    let ledger = VerificationLedger::new(dir.path());
    let entry = make_verification_entry("gate-rt", false);
    ledger.append(&entry).unwrap();
    let loaded = ledger.load_all().unwrap();
    assert_eq!(loaded[0].gate_name, entry.gate_name);
    assert_eq!(loaded[0].passed, entry.passed);
    assert_eq!(loaded[0].failure_details, entry.failure_details);
    assert_eq!(loaded[0].evidence_summary, entry.evidence_summary);
    assert_eq!(loaded[0].timestamp, entry.timestamp);
}
