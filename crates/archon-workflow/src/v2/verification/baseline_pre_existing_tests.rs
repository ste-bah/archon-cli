//! The typed-names / routing-table cross-check, unit by unit.
use serde_json::json;

use super::{declared_failed_names, routed_to_other_tasks};
use crate::v2::verification::baseline_rule::{BaselineStamp, OtherOwnerTest};

const MINE: &str = "gate::tests::mine";
const THEIRS: &str = "stooq::tests::theirs";

fn stamp(routed: &[(&str, &str)]) -> BaselineStamp {
    BaselineStamp {
        tasks: vec!["TASK-A".into()],
        other_owner: routed
            .iter()
            .map(|(test_id, owner_task)| OtherOwnerTest {
                test_id: (*test_id).into(),
                owner_task: (*owner_task).into(),
            })
            .collect(),
        ..Default::default()
    }
}

fn names(ids: &[&str]) -> Vec<String> {
    ids.iter().map(|id| (*id).to_string()).collect()
}

#[test]
fn a_well_formed_array_of_names_is_read_and_anything_else_is_malformed() {
    assert_eq!(
        declared_failed_names(&json!({"matched_test_check_names": {"failed": [THEIRS, MINE]}})),
        Some(names(&[THEIRS, MINE]))
    );
    assert_eq!(
        declared_failed_names(&json!({"matched_test_check_names": {"failed": []}})),
        Some(Vec::new())
    );
    for malformed in [
        json!({}),
        json!({"matched_test_check_names": {}}),
        json!({"matched_test_check_names": {"failed": null}}),
        json!({"matched_test_check_names": {"failed": "a, b"}}),
        json!({"matched_test_check_names": {"failed": [THEIRS, 7]}}),
        json!({"matched_test_check_names": {"failed": [THEIRS, "   "]}}),
    ] {
        assert_eq!(declared_failed_names(&malformed), None, "{malformed}");
    }
}

#[test]
fn only_names_every_one_of_which_is_routed_to_another_task_are_covered() {
    let routed = stamp(&[(THEIRS, "TASK-B"), (MINE, "TASK-A")]);
    assert!(routed_to_other_tasks(&names(&[THEIRS]), &routed));
    // Routed to the task under verification.
    assert!(!routed_to_other_tasks(&names(&[MINE]), &routed));
    assert!(!routed_to_other_tasks(&names(&[THEIRS, MINE]), &routed));
    // In no routing entry at all.
    assert!(!routed_to_other_tasks(
        &names(&["plan::tests::absent"]),
        &routed
    ));
    // Nothing named.
    assert!(!routed_to_other_tasks(&[], &routed));
    // A second entry for the same name routes it back to this task.
    let both = stamp(&[(THEIRS, "TASK-B"), (THEIRS, "TASK-A")]);
    assert!(!routed_to_other_tasks(&names(&[THEIRS]), &both));
    // The stamp does not know which task is under verification.
    let unknown = BaselineStamp {
        tasks: Vec::new(),
        ..stamp(&[(THEIRS, "TASK-B")])
    };
    assert!(!routed_to_other_tasks(&names(&[THEIRS]), &unknown));
}
