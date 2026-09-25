//! The matcher against the entry shapes the live task files actually carry.
use super::ForbiddenPaths;

fn forbidden(entries: &[&str]) -> ForbiddenPaths {
    ForbiddenPaths::from_entries(entries.iter().copied())
}

#[test]
fn an_exact_file_matches_itself_by_either_spelling_and_nothing_else() {
    let f = forbidden(&["crates/x/src/gate.rs"]);
    assert!(f.matches("crates/x/src/gate.rs"));
    assert!(f.matches("./crates/x/src/gate.rs"));
    assert!(!f.matches("crates/x/src/gate_tests.rs"));
    assert!(!f.matches("crates/x/src/gate.rs/inner"));
    assert!(!f.matches("other/crates/x/src/gate.rs"));
}

#[test]
fn a_directory_prefix_by_slash_star_or_double_star_covers_the_subtree() {
    for entry in ["docs/", "docs/*", "docs/**", "`docs/**`"] {
        let f = forbidden(&[entry]);
        assert!(f.matches("docs/a.md"), "{entry}");
        assert!(f.matches("docs/deep/b.md"), "{entry}");
        assert!(f.matches("docs"), "{entry}");
        assert!(!f.matches("docs2/a.md"), "{entry}");
        assert!(!f.matches("src/docs/a.md"), "{entry}");
        assert_eq!(f.patterns(), vec!["docs/".to_string()], "{entry}");
    }
}

#[test]
fn a_glob_spans_directories_and_a_bare_basename_matches_anywhere() {
    let f = forbidden(&["providers/*.rs", "crates/x/src/providers*"]);
    assert!(f.matches("providers/a.rs"));
    assert!(f.matches("providers/nested/b.rs"));
    assert!(!f.matches("providers/a.md"));
    assert!(f.matches("crates/x/src/providers.rs"));
    assert!(f.matches("crates/x/src/providers/tv.rs"));
    assert!(!f.matches("crates/x/src/provider_capability.rs"));
    let bare = forbidden(&["coverage.rs"]);
    assert!(bare.matches("crates/x/src/coverage.rs"));
    assert!(bare.matches("coverage.rs"));
    assert!(!bare.matches("crates/x/src/test_coverage.rs"));
    assert_eq!(bare.patterns(), vec!["**/coverage.rs".to_string()]);
}

/// The live bullet shapes: several backticked paths in one entry, a wrap
/// joined after a `/`, prose before and after, a trailing task reference.
#[test]
fn a_prose_entry_yields_every_backticked_path_and_ignores_the_prose() {
    let f = forbidden(&[
        "`crates/x/src/gate.rs` and every other crate module: `data_lake.rs`, \
         `data_store.rs`, `validation.rs` (TASK-DL-002… 009) — defects are reported, never fixed.",
        "Frozen chain: `tasks/PRD-ONE/*` and `prds/PRD-ONE.md`.",
        "The consumed spec: `.archon/lab/strategies/v1/ strategy-spec.json` and `evidence/**`.",
        "Frozen chain",
        "tests, PRD files",
    ]);
    assert!(f.matches("crates/x/src/gate.rs"));
    assert!(f.matches("crates/x/src/data_lake.rs"));
    assert!(f.matches("crates/x/src/data_store.rs"));
    assert!(!f.matches("crates/x/src/data_store/validation_tests.rs"));
    assert!(f.matches("tasks/PRD-ONE/TASK-1.md"));
    assert!(f.matches("prds/PRD-ONE.md"));
    assert!(!f.matches("prds/PRD-TWO.md"));
    assert!(f.matches(".archon/lab/strategies/v1/strategy-spec.json"));
    assert!(f.matches("evidence/run.json"));
    assert!(!f.matches("chain"));
    assert!(!f.matches("tests"));
    assert_eq!(f.len(), 8, "{:?}", f.patterns());
}

#[test]
fn an_unquoted_entry_is_cut_at_its_prose_and_quotes_and_dot_slash_are_stripped() {
    let f = forbidden(&[
        "src/lib.rs — the public surface",
        "\"./src/main.rs\" (entry point)",
        "'Cargo.lock',",
    ]);
    assert_eq!(
        f.patterns(),
        vec![
            "src/lib.rs".to_string(),
            "src/main.rs".to_string(),
            "**/Cargo.lock".to_string()
        ]
    );
    assert!(f.matches("src/lib.rs"));
    assert!(f.matches("src/main.rs"));
    assert!(f.matches("Cargo.lock"));
}

#[test]
fn wildcard_only_and_empty_entries_forbid_nothing() {
    let f = forbidden(&["*", "**", "/", "", "   ", "`*`"]);
    assert!(f.is_empty());
    assert!(!f.matches("anything.rs"));
    assert!(!ForbiddenPaths::default().matches("src/lib.rs"));
}

#[test]
fn the_wire_form_reads_back_to_the_same_matcher() {
    let f = forbidden(&["docs/**", "src/a.rs", "coverage.rs", "providers/*.rs"]);
    let again = ForbiddenPaths::from_entries(f.patterns());
    assert_eq!(again, f);
    assert_eq!(again.patterns(), f.patterns());
}

#[test]
fn describe_is_bounded() {
    let entries: Vec<String> = (0..45).map(|i| format!("src/f{i}.rs")).collect();
    let f = ForbiddenPaths::from_entries(&entries);
    let text = f.describe();
    assert!(text.starts_with("src/f0.rs, src/f1.rs"), "{text}");
    assert!(text.ends_with(" …and 5 more"), "{text}");
    assert!(!text.contains("src/f44.rs"), "{text}");
    assert_eq!(forbidden(&["a/b.rs", "c/"]).describe(), "a/b.rs, c/");
}

/// Only a pattern that names nothing outside the declared paths is dropped;
/// one that merely contains or intersects a declared path keeps freezing
/// everything else it names.
#[test]
fn only_a_pattern_wholly_inside_a_declared_path_is_dropped() {
    let forbidden = ForbiddenPaths::from_entries([
        "`crates/b/src/lib.rs` (sibling's own file)",
        "`crates/b/src/gen/` (inside a declared scope)",
        "`crates/b/src/gen/*.rs`",
        "coverage.rs",
        "`crates/engine/src/`",
        "`crates/b/tests/` (frozen tests)",
        "`crates/*/Cargo.toml`",
        "`**/*_test.rs`",
    ]);
    let kept = forbidden.without_within([
        "crates/b/src/lib.rs",
        "crates/b/src/gen/",
        "crates/b/src/coverage.rs",
        "crates/engine/src/new.rs",
        "crates/b/tests/one_test.rs",
        "crates/b/Cargo.toml",
    ]);
    assert_eq!(
        kept.patterns(),
        vec![
            "**/coverage.rs",
            "crates/engine/src/",
            "crates/b/tests/",
            "crates/*/Cargo.toml",
            "*/*_test.rs",
        ]
    );
    // The siblings stay frozen...
    for frozen in [
        "crates/a/src/coverage.rs",
        "crates/engine/src/lib.rs",
        "crates/b/tests/frozen.rs",
        "crates/a/Cargo.toml",
        "crates/a/src/x_test.rs",
    ] {
        assert!(kept.matches(frozen), "{frozen} must stay forbidden");
    }
    // ...and the declared files are the ones the guard and capture exempt.
    assert!(!kept.matches("crates/b/src/lib.rs"));
    assert!(forbidden.without_within(Vec::<String>::new()) == forbidden);
}
