use super::*;

#[test]
fn review_examples_use_the_opening_subject_and_shared_path_normalization() {
    let prose = task(
        "TASK-X-010",
        "REQ-X-001",
        "## Files Expected to Change\n\n- `state_load` updates `src/a.rs`\n\n## Files Forbidden to Change\n\n- src/a.rs\n\n## Scope\n\n- REQ-X-001 lives in `present_symbol` in `src/lib.rs`.\n",
    );
    let trace = run(&[("TASK-X-010", &prose)]);
    assert!(
        trace
            .findings
            .iter()
            .all(|finding| !finding.text.contains("declared `src/a.rs`")),
        "{}",
        trace.report
    );

    for declared in ["absolute", "dotfile"] {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("repo");
        std::fs::create_dir_all(&root).unwrap();
        let base = repository(&root);
        let declared_path = if declared == "absolute" {
            format!("{}/src/a.rs", root.display())
        } else {
            ".mcp.json".to_string()
        };
        let forbidden = if declared == "absolute" {
            "src/a.rs"
        } else {
            ".mcp.json"
        };
        let body = task(
            "TASK-X-010",
            "REQ-X-001",
            &format!(
                "## Files Expected to Change\n\n- {declared_path} — exists\n\n## Files Forbidden to Change\n\n- {forbidden}\n\n## Scope\n\n- REQ-X-001 lives in `present_symbol` in `src/lib.rs`.\n"
            ),
        );
        let task_dir = task_set(dir.path(), &root, &base, &[("TASK-X-010", &body)]);
        let (bindings, findings) =
            crate::command::requirement_trace::load_bindings_with_findings(&task_dir).unwrap();
        assert!(findings.is_empty(), "{findings:?}");
        let trace = falsify_claims(&task_dir, bindings).expect("falsify");
        assert!(
            trace.findings.iter().any(|finding| {
                finding.text.contains(&format!(
                    "declared `{}`",
                    if declared == "absolute" {
                        "src/a.rs"
                    } else {
                        ".mcp.json"
                    }
                ))
            }),
            "{}",
            trace.report
        );
    }
}
