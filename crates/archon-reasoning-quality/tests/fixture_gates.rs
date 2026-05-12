use std::path::PathBuf;

use archon_reasoning_quality::{
    audit_labeled_turns, evaluate_labeled_turns, fixtures::load_labeled_turns,
};

#[test]
fn labeled_turn_fixtures_pass_quality_gates_and_audit() {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("labeled_turns");
    let fixtures = load_labeled_turns(&dir).expect("load fixtures");
    let audit = audit_labeled_turns(&fixtures);
    assert!(audit.passed(), "fixture audit failed: {audit:?}");

    let evaluation = evaluate_labeled_turns(&fixtures);
    assert!(
        evaluation.gates_pass(),
        "extractor gates failed: {evaluation:?}"
    );
}
