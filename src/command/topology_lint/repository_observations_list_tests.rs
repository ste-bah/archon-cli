use super::*;

#[test]
fn observations_do_not_extend_the_declaration_list() {
    assert_eq!(
        paths_in_item("`src/lib.rs` — exists (1 line), `src/other.rs`"),
        ["src/lib.rs"]
    );
}

#[test]
fn wrapped_heads_continue_after_list_joiners() {
    for ending in [",", "and", "and/or", "+"] {
        assert_eq!(
            paths_in_item(&format!("`src/a.rs` {ending}\n `src/b.rs` (create)")),
            ["src/a.rs", "src/b.rs"]
        );
    }
}

#[test]
fn malformed_observations_never_declare_description_basenames() {
    for description in [
        "exists (about 12 lines); update `report.rs`",
        "not observed; add `validation_tests.rs`",
    ] {
        assert_eq!(
            paths_in_item(&format!("`source/report.rs` — {description}")),
            ["source/report.rs"]
        );
    }
}

#[test]
fn symbols_and_globs_are_not_literal_declarations() {
    for symbol in ["Widget", "Widget::method", "call()", "mod helper;"] {
        assert!(paths_in_item(&format!("`{symbol}` with `src/widget.rs`")).is_empty());
    }
    for glob in ["src/*.rs", "src/?.rs", "src/[ab].rs"] {
        assert!(paths_in_item(glob).is_empty());
    }
    assert_eq!(
        paths_in_item("`src/widgets/` — update `mod.rs`"),
        ["src/widgets/"]
    );
    assert_eq!(
        paths_in_item("`src/a.rs`, and `src/b.rs` and `src/c.rs`: description `d.rs`"),
        ["src/a.rs", "src/b.rs", "src/c.rs"]
    );
}

#[test]
fn fenced_examples_do_not_declare_files_or_headings() {
    let raw = body(
        "[]",
        "- `src/lib.rs`\n\n~~~~markdown\n## Files Expected to Change\n- `fake.rs`\n~~~~\n\n- `src/new.rs`",
    );
    let task =
        archon_workflow::task_universe::parsing::parse_task_file(Path::new("TASK-X-001.md"), &raw)
            .unwrap();
    assert_eq!(deliverable_paths(&task, &raw), ["src/lib.rs", "src/new.rs"]);
}
