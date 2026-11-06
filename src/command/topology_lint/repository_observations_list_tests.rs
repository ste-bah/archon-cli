use super::*;

fn declared(head: &str) -> Vec<String> {
    let raw = body("[]", head);
    let task =
        archon_workflow::task_universe::parsing::parse_task_file(Path::new("TASK-X-001.md"), &raw)
            .unwrap();
    deliverable_paths(&task, &raw)
}

#[test]
fn live_heads_declare_only_the_opening_path() {
    let (_temp, _project, _tasks, tree) = grounded();
    for opening in [
        "source/report.rs".to_string(),
        tree.root().join("source/report.rs").display().to_string(),
    ] {
        for description in [
            "exists (directory `capability/` exists; `report.rs` observed elsewhere)",
            "exists (about 12 lines); update `validation_tests.rs`",
        ] {
            assert_eq!(
                declared(&format!("- `{opening}` — {description}")),
                [opening.replace('\\', "/")]
            );
        }
    }
}

#[test]
fn declarations_match_the_ownership_reader() {
    for head in [
        "- `src/a.rs` and `src/b.rs`",
        "- Likely anchors: `src/a.rs`, `src/b.rs`",
        "- `src/widgets/` — update `mod.rs`",
        "- src/a.rs.",
        "- `Widget` with `src/widget.rs`",
    ] {
        assert_eq!(
            declared(head),
            archon_write_plan::declared_paths::declared_file_paths(head),
            "{head}"
        );
    }
}

#[test]
fn declaration_sources_include_contracts_and_shared_targets() {
    let raw = body(
        "[{kind: source, artifact_path: src/existing.rs}]",
        "- `src/lib.rs`",
    )
    .replace(
        "required_tools: []",
        "required_tools: []\nshared_append_target_files: [src/shared.rs]",
    );
    let task =
        archon_workflow::task_universe::parsing::parse_task_file(Path::new("TASK-X-001.md"), &raw)
            .unwrap();
    assert_eq!(
        deliverable_paths(&task, &raw),
        ["src/existing.rs", "src/lib.rs", "src/shared.rs"]
    );
}

#[test]
fn observation_lint_uses_prose_fences_without_changing_shared_sections() {
    assert_eq!(
        declared(
            "- `src/lib.rs`\n\n```markdown\n## Files Expected to Change\n- `fake.rs`\n```\n\n- `src/new.rs`"
        ),
        ["src/lib.rs", "src/new.rs"]
    );
}
