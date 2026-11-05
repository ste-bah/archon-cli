use super::*;

fn inspect_result(tasks: &Path, project: &Path, raw: &str) -> anyhow::Result<Vec<String>> {
    crate::command::topology_lint::repository_claims::inspect(
        project,
        tasks,
        "TASK-X-001",
        &tasks.join("TASK-X-001.md"),
        raw,
    )
}

#[test]
fn lexical_escapes_are_body_findings() {
    let (_temp, project, tasks, tree) = grounded();
    let outside = project.join("outside.rs").display().to_string();
    for path in ["../x", "src/../x.rs", outside.as_str()] {
        let error = observe(&tree, path).unwrap_err();
        assert!(
            error.to_string().contains("path leaves the repository"),
            "{error}"
        );
        let raw = body("[]", &format!("- `{path}`"));
        let found = inspect_result(&tasks, &project, &raw).unwrap();
        assert_eq!(found.len(), 1, "{found:?}");
        assert!(found[0].contains(path), "{found:?}");
        assert!(found[0].contains("path leaves the repository"), "{found:?}");
        if Path::new(path).is_absolute() {
            let raw = body("[]", "- `src/lib.rs`") + &format!("\n## Notes\n`{path}` exists.\n");
            assert!(
                inspect_result(&tasks, &project, &raw)
                    .unwrap()
                    .iter()
                    .any(|finding| finding.contains("path leaves the repository"))
            );
        }
    }
}

#[cfg(unix)]
#[test]
fn symlink_escapes_are_body_findings_in_both_lints() {
    let (_temp, project, tasks, tree) = grounded();
    std::fs::write(project.join("outside.rs"), "secret\n").unwrap();
    std::os::unix::fs::symlink(&project, tree.root().join("escape")).unwrap();
    for path in ["escape/", "escape/outside.rs", "escape/missing.rs"] {
        let raw = body("[]", &format!("- `{path}`"));
        let found = inspect_result(&tasks, &project, &raw).unwrap();
        assert_eq!(found.len(), 1, "{found:?}");
        assert!(found[0].contains("path leaves the repository"), "{found:?}");
        let raw = body("[]", "- `src/lib.rs`") + &format!("\n## Notes\n`{path}` exists.\n");
        assert!(
            inspect_result(&tasks, &project, &raw)
                .unwrap()
                .iter()
                .any(|finding| finding.contains("path leaves the repository"))
        );
        assert!(
            observe(&tree, path)
                .unwrap_err()
                .to_string()
                .contains("path leaves the repository")
        );
    }
}

#[test]
fn oversized_files_are_named_operational_errors() {
    let (_temp, project, tasks, tree) = grounded();
    let path = tree.root().join("oversized.rs");
    std::fs::File::create(&path)
        .unwrap()
        .set_len(64 * 1024 * 1024 + 1)
        .unwrap();
    let raw = body("[]", "- `oversized.rs`");
    let error = inspect_result(&tasks, &project, &raw).unwrap_err();
    assert!(error.to_string().contains("oversized.rs"), "{error}");
    assert!(error.to_string().contains("byte limit"), "{error}");
    assert!(
        error
            .downcast_ref::<super::super::super::repository_paths::PathFault>()
            .is_none()
    );
}

#[test]
fn malformed_paths_do_not_abort_the_set_gate() {
    let (_temp, project, tasks, _tree) = grounded();
    let bad = "src/lib.rs/x.rs";
    let raw = body(
        "[]",
        &format!("- `{bad}`\n- `src/existing.rs` — exists (2 lines)"),
    );
    std::fs::write(tasks.join("TASK-X-001.md"), &raw).unwrap();
    let found =
        crate::command::topology_lint::repository_claims::set_findings(&project, &tasks).unwrap();
    assert_eq!(found.len(), 2, "{found:?}");
    let other = body("[]", "- `src/lib.rs` — exists (5 lines)").replace("TASK-X-001", "TASK-X-002");
    std::fs::write(tasks.join("TASK-X-002.md"), other).unwrap();
    let found =
        crate::command::topology_lint::repository_claims::set_findings(&project, &tasks).unwrap();
    assert_eq!(found.len(), 3, "{found:?}");
    let found = inspect_result(&tasks, &project, &raw).unwrap();
    assert!(
        found.iter().any(|finding| finding.contains(bad)),
        "{found:?}"
    );
    let raw = body("[]", "- `src/lib.rs`") + &format!("\n## Notes\n`{bad}` exists.\n");
    assert_eq!(inspect_result(&tasks, &project, &raw).unwrap().len(), 1);
}

#[test]
fn streaming_counts_preserve_empty_and_unterminated_lines() {
    let (_temp, _project, _tasks, tree) = grounded();
    for bytes in [b"".as_slice(), b"a", b"a\n", b"a\n\nb", b"\n\n"] {
        std::fs::write(tree.root().join("count.rs"), bytes).unwrap();
        assert_eq!(
            observe(&tree, "count.rs").unwrap(),
            Observation::File {
                lines: count_lines(bytes)
            }
        );
    }
}

#[test]
fn unavailable_repository_root_is_an_operational_fault() {
    let (_temp, _project, _tasks, tree) = grounded();
    let moved = tree.root().with_file_name("moved-repository");
    std::fs::rename(tree.root(), &moved).unwrap();
    let result = observe(&tree, "src/lib.rs");
    std::fs::rename(&moved, tree.root()).unwrap();
    let error = result.unwrap_err();
    assert!(error.to_string().contains("repository root"), "{error:#}");
    assert!(
        error
            .downcast_ref::<super::super::super::repository_paths::PathFault>()
            .is_none()
    );
}

#[cfg(unix)]
#[test]
fn cyclic_symlinks_are_body_faults() {
    let (_temp, project, tasks, tree) = grounded();
    std::os::unix::fs::symlink("cycle.rs", tree.root().join("cycle.rs")).unwrap();
    let raw = body("[]", "- `cycle.rs`");
    let found = inspect_result(&tasks, &project, &raw).unwrap();
    assert_eq!(found.len(), 1, "{found:?}");
    assert!(found[0].contains("cycle.rs"), "{found:?}");
}

#[test]
fn a_directory_claim_about_a_file_remains_a_finding() {
    let (_temp, project, tasks, tree) = grounded();
    let raw = body("[]", "- `src/lib.rs` — exists (directory)");
    let found = findings(&tasks, &project, &tree, &raw);
    assert_eq!(found.len(), 1, "{found:?}");
    assert!(found[0].contains("is a file of 1 lines"), "{found:?}");
}
