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

fn findings(text: &str) -> Vec<String> {
    let (_temp, project, _tasks, tree) = grounded();
    findings_against(&tree, &project, "TASK-X-425", text)
}

// A: a negated bare name with no explicit scope names no repository path.

#[test]
fn a_negated_bare_name_without_a_scope_is_not_a_root_claim() {
    let text = "No `Cargo.toml` is present in the new crate.";
    assert!(claims(text).is_empty(), "{:?}", claims(text));
    let found = findings(text);
    assert!(found.is_empty(), "{found:?}");
}

#[test]
fn a_negated_list_without_a_scope_keeps_only_its_slashed_paths() {
    let found = findings("No `src/existing.rs` or `Cargo.toml` exists.");
    assert_eq!(found.len(), 1, "{found:?}");
    assert!(found[0].contains("`src/existing.rs` does not exist"));
}

// B: a scope cited by base name for a nested file is that file, not a
// missing root directory.

#[test]
fn a_symbol_in_a_nested_file_cited_by_base_name_is_no_claim() {
    let found = findings("`promotion_gates.live` exists in `strategy_spec.rs`.");
    assert!(found.is_empty(), "{found:?}");
}

#[test]
fn the_live_sentence_with_a_nested_file_is_no_claim() {
    let found = findings(
        "... — pinned by the existing `promotion_gates.live` assertion in `strategy_spec.rs`; no spec surface enables live trading.",
    );
    assert!(found.is_empty(), "{found:?}");
}

#[test]
fn a_scope_cited_by_a_trailing_suffix_of_a_nested_file_is_no_claim() {
    let found = findings("`promotion_gates.live` exists in `src/strategy_spec.rs`.");
    assert!(found.is_empty(), "{found:?}");
}

#[test]
fn a_file_like_scope_absent_everywhere_is_no_claim() {
    let found = findings("`foo.rs` exists in `config.json`.");
    assert!(found.is_empty(), "{found:?}");
}

#[test]
fn a_unique_nested_directory_cited_by_base_name_is_that_directory() {
    let present = findings("`w.rs` exists in `widgets/`.");
    assert!(present.is_empty(), "{present:?}");
    let missing = findings("`ghost.rs` exists in `widgets/`.");
    assert_eq!(missing.len(), 1, "{missing:?}");
    assert!(missing[0].contains("`crates/x/src/widgets/ghost.rs` exists"));
}

#[test]
fn an_ambiguous_nested_directory_is_no_claim() {
    // `crate/tests` and `crate/tests/tests` both end with `/tests`.
    let found = findings("`ghost.rs` exists in `tests/`.");
    assert!(found.is_empty(), "{found:?}");
}

#[test]
fn a_missing_directory_scope_is_still_checked() {
    let found = findings("`a.rs` exists under `missing/`.");
    assert_eq!(found.len(), 1, "{found:?}");
    assert!(found[0].contains("`missing/a.rs` exists"));
}

// C: filler words between the path and its predicate keep the scope.

#[test]
fn currently_between_path_and_scoped_predicate_keeps_the_scope() {
    let found = findings("`foo.rs` currently exists in `src/`.");
    assert_eq!(found.len(), 1, "{found:?}");
    assert!(found[0].contains("`src/foo.rs` exists"));
}

#[test]
fn module_between_path_and_scoped_predicate_keeps_the_scope() {
    let found = findings("`foo.rs` module exists under `src/`.");
    assert_eq!(found.len(), 1, "{found:?}");
    assert!(found[0].contains("`src/foo.rs` exists"));
}

#[test]
fn a_dash_between_path_and_scoped_predicate_keeps_the_scope() {
    let found = findings("`foo.rs` — exists in `src/`.");
    assert_eq!(found.len(), 1, "{found:?}");
    assert!(found[0].contains("`src/foo.rs` exists"));
}

#[test]
fn a_scope_keeps_its_case() {
    assert_eq!(
        extract_claims("`a.rs` exists in `Docs/V2/`.")
            .into_iter()
            .map(|claim| claim.scope)
            .collect::<Vec<_>>(),
        vec![Some("Docs/V2".to_string())]
    );
}
