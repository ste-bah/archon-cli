use super::*;
use crate::write_coordinator::coordinator::{CoordinatedOutcome, PlanRecord};
use crate::write_coordinator::patch_manifest::{COMPLEXITY_SCAN_UNRELIABLE, UnreliableScan};

#[test]
fn coordination_outcome_row_records_unreliable_complexity_scans() {
    let dir = tempfile::tempdir().expect("dir");
    let store = WorkflowStore::project(dir.path());
    let note = UnreliableScan {
        rule: COMPLEXITY_SCAN_UNRELIABLE.to_string(),
        path: "src/a.rs".to_string(),
        line: 7,
        language: "rust".to_string(),
        reason: "post-patch text: syntax error inside function 'f'".to_string(),
    };
    let outcome = CoordinatedOutcome {
        run_id: "run1".into(),
        stage_id: "implement".into(),
        plans: vec![PlanRecord {
            item_id: "i0".into(),
            wave_id: 0,
            work_unit_ids: vec![],
            target_files: vec!["src/a.rs".into()],
            changed_files: vec!["src/a.rs".into()],
            post_hashes: Default::default(),
            patch_bytes_len: 1,
            complexity_scan_unreliable: vec![note],
        }],
        ..Default::default()
    };
    record_write_coordination_outcome(&store, &outcome).expect("recorded");
    let path = store
        .run_dir("run1")
        .join("learning/write-coordination/outcomes.jsonl");
    let row: serde_json::Value =
        serde_json::from_str(std::fs::read_to_string(path).expect("row").trim()).expect("json");
    let recorded = &row[COMPLEXITY_SCAN_UNRELIABLE][0];
    assert_eq!(recorded["path"], "src/a.rs");
    assert_eq!(recorded["line"], 7);
    assert_eq!(recorded["language"], "rust");
    assert_eq!(recorded["rule"], COMPLEXITY_SCAN_UNRELIABLE);
}
