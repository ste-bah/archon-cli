use super::*;

#[test]
fn scoped_candidates_are_resolved_from_the_repository_tree() {
    let (_temp, project, _tasks, tree) = grounded();

    assert!(
        findings_against(
            &tree,
            &project,
            "TASK-X-001",
            "`foo.rs` exists in `docs/v1.2`."
        )
        .is_empty()
    );

    let missing = findings_against(
        &tree,
        &project,
        "TASK-X-001",
        "`missing.rs` exists in `docs/v1.2`.",
    );
    assert_eq!(missing.len(), 1, "{missing:?}");
    assert!(missing[0].contains("`docs/v1.2/missing.rs` exists"));

    let single_word_scope = findings_against(
        &tree,
        &project,
        "TASK-X-001",
        "`missing.rs` exists under `src`.",
    );
    assert_eq!(single_word_scope.len(), 1, "{single_word_scope:?}");
    assert!(single_word_scope[0].contains("`src/missing.rs` exists"));

    assert!(
        findings_against(
            &tree,
            &project,
            "TASK-X-001",
            "`foo.rs` exists in `src/lib.rs`."
        )
        .is_empty()
    );

    let live_sentence = "... — pinned by the existing `promotion_gates.live` assertion in `strategy_spec.rs`; no spec surface enables live trading.";
    assert!(findings_against(&tree, &project, "TASK-X-001", live_sentence).is_empty());

    for text in [
        "No `existing.rs` or `missing.rs` exists under `src/`.",
        "Neither `existing.rs` nor `missing.rs` exist in `src/`.",
    ] {
        let findings = findings_against(&tree, &project, "TASK-X-001", text);
        assert_eq!(findings.len(), 1, "{text:?}: {findings:?}");
        assert!(findings[0].contains("`src/existing.rs` does not exist"));
    }
}
