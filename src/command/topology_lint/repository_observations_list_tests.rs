use super::*;

#[test]
fn paths_after_an_observation_on_the_same_line_are_still_checked() {
    let (_temp, project, tasks, tree) = grounded();
    let raw = body(
        "[]",
        "- `src/lib.rs` — exists (1 line), `src/existing.rs` — exists (3 lines)",
    );
    assert!(findings(&tasks, &project, &tree, &raw).is_empty());

    let raw = body("[]", "- `src/lib.rs` — exists (1 line); `src/missing_n.rs`");
    let found = findings(&tasks, &project, &tree, &raw);
    assert_eq!(found.len(), 1, "{found:?}");
    assert!(
        found[0].contains("deliverable path `src/missing_n.rs` has no verifiable observation"),
        "{}",
        found[0]
    );
}

#[test]
fn wrapped_heads_continue_after_and_or_and_plus() {
    let (_temp, project, tasks, tree) = grounded();
    for ending in ["and/or", "+"] {
        let raw = body(
            "[]",
            &format!("- `src/lib.rs` {ending}\n  `src/missing.rs` (create)"),
        );
        let found = findings(&tasks, &project, &tree, &raw);
        assert_eq!(found.len(), 2, "ending={ending:?}: {found:?}");
        assert!(
            found
                .iter()
                .any(|finding| finding.contains("src/missing.rs")),
            "ending={ending:?}: {found:?}"
        );
    }
}
