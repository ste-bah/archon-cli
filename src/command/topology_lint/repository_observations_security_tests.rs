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
    for path in ["../x", "src/../x.rs"] {
        let error = observe(&tree, path).unwrap_err();
        assert!(
            error.to_string().contains("path leaves the repository"),
            "{error}"
        );
        let raw = body("[]", &format!("- `{path}`"));
        let found = inspect_result(&tasks, &project, &raw).unwrap();
        assert_eq!(found.len(), 1, "{found:?}");
        assert!(found[0].contains(path), "{found:?}");
    }
    let outside = project.join("outside.rs").display().to_string();
    for suffix in ["", " — exists (4 lines)", " — absent"] {
        let raw = body("[]", &format!("- `{outside}`{suffix}"))
            + &format!("\n## Notes\n`{outside}` exists.\n");
        assert!(inspect_result(&tasks, &project, &raw).unwrap().is_empty());
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
fn oversized_files_only_count_under_explicit_claims() {
    let (_temp, project, tasks, tree) = grounded();
    let path = tree.root().join("oversized.rs");
    std::fs::File::create(&path)
        .unwrap()
        .set_len(64 * 1024 * 1024 + 1)
        .unwrap();
    for suffix in ["", " — exists (directory)", " — absent"] {
        let raw = body("[]", &format!("- `./oversized.rs`{suffix}"));
        let found = inspect_result(&tasks, &project, &raw).unwrap();
        if suffix.is_empty() {
            assert!(found.is_empty(), "{found:?}");
        }
        assert!(
            !found.iter().any(|finding| finding.contains("byte limit")),
            "{found:?}"
        );
    }
    let raw = body("[]", "- `./oversized.rs` — exists (1 line)");
    let found = inspect_result(&tasks, &project, &raw).unwrap();
    assert_eq!(found.len(), 1, "{found:?}");
    assert!(
        found[0].contains("oversized.rs") && found[0].contains("byte limit"),
        "{found:?}"
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
    assert!(found[0].contains("is a file"), "{found:?}");
}

#[test]
fn bare_observations_need_explicit_scope() {
    let (_temp, project, tasks, tree) = grounded();
    let raw = body("[]", "- `src/lib.rs`") + "\n## Notes\n`missing.rs` — exists (12 lines)\n";
    assert!(inspect_result(&tasks, &project, &raw).unwrap().is_empty());
    let raw = body("[]", "- `src/lib.rs`")
        + "\n## Notes\n`existing.rs` — exists (2 lines) under `src/`\n";
    let found = inspect_result(&tasks, &project, &raw).unwrap();
    assert_eq!(found.len(), 1, "{found:?}");
    assert!(found[0].contains("has 3 lines"), "{found:?}");
    assert_eq!(
        observe(&tree, "src/existing.rs").unwrap(),
        Observation::File { lines: 3 }
    );
}

#[test]
fn directory_heading_scopes_component_observations() {
    let (_temp, project, tasks, tree) = recorded("PRD-X", |repo| {
        std::fs::create_dir_all(repo.join("src/providers/capability_tests")).unwrap();
        std::fs::write(
            repo.join("src/providers/capability_tests/engine_tests.rs"),
            "//\n".repeat(4),
        )
        .unwrap();
    });
    let raw = body("[]", "")
        + "\n## Tests in `src/providers/`\n`capability_tests/engine_tests.rs` — exists (4 lines)\n";
    for raw in [&raw, &raw.replace("(4 lines)", "(3 lines)")] {
        let found = inspect_result(&tasks, &project, raw).unwrap();
        assert_eq!(found.len(), 1, "{found:?}");
        assert!(
            found[0].contains("is not repository-root relative"),
            "{found:?}"
        );
        assert!(found[0].contains("src/providers/capability_tests/engine_tests.rs"));
    }
    assert!(tree.root().is_dir());
}

#[test]
fn path_faults_dedupe_normalized_spellings() {
    let (_temp, project, tasks, tree) = grounded();
    let absolute = tree
        .root()
        .join("src/lib.rs/child.rs")
        .display()
        .to_string();
    let raw = body(
        "[]",
        &format!("- `./src/lib.rs/child.rs`\n- `src//lib.rs/child.rs`\n- `{absolute}`"),
    ) + "\n## Notes\n`src/lib.rs/child.rs` exists.\n`./src/lib.rs/child.rs` exists.\n";
    let found = inspect_result(&tasks, &project, &raw).unwrap();
    assert_eq!(found.len(), 1, "{found:?}");
}

#[cfg(unix)]
#[test]
fn outside_absolute_aliases_are_skipped_without_resolving_candidates() {
    let (_temp, project, tasks, tree) = grounded();
    let alias = project.join("repository-alias");
    std::os::unix::fs::symlink(tree.root(), &alias).unwrap();
    for (path, count) in [("src/existing.rs", 2), ("missing/deep/file.rs", 1)] {
        let named = alias.join(path).display().to_string();
        let raw = body("[]", &format!("- `{named}` — exists ({count} lines)"));
        let found = inspect_result(&tasks, &project, &raw).unwrap();
        assert!(found.is_empty(), "{found:?}");
    }
}

#[test]
fn outside_case_aliases_are_skipped() {
    let (_temp, project, tasks, tree) = grounded();
    let alias = tree.root().with_file_name("REPO");
    // A case-sensitive filesystem has no alternate spelling for this root.
    if !alias.exists() {
        return;
    }
    assert_eq!(
        alias.canonicalize().unwrap(),
        tree.root().canonicalize().unwrap()
    );
    let named = alias.join("missing/deep/file.rs").display().to_string();
    let raw = body("[]", &format!("- `{named}` — exists (1 line)"));
    let found = inspect_result(&tasks, &project, &raw).unwrap();
    assert!(found.is_empty(), "{found:?}");
}

#[test]
fn absolute_temp_paths_require_a_recognized_root_spelling() {
    let (temp, project, tasks, tree) = grounded();
    let named = temp
        .path()
        .join("repo/src/existing.rs")
        .display()
        .to_string();
    let raw = body("[]", &format!("- `{named}` — exists (2 lines)"));
    let found = inspect_result(&tasks, &project, &raw).unwrap();
    if Path::new(&named).starts_with(tree.root()) {
        assert_eq!(found.len(), 1, "{found:?}");
        assert!(found[0].contains("has 3 lines"), "{found:?}");
    } else {
        assert!(found.is_empty(), "{found:?}");
    }
}

#[cfg(unix)]
#[test]
fn absolute_symlink_escape_is_a_body_fault_only_under_a_recognized_root() {
    let (_temp, project, tasks, tree) = grounded();
    let alias = project.join("repository-alias");
    std::os::unix::fs::symlink(tree.root(), &alias).unwrap();
    std::os::unix::fs::symlink(&project, tree.root().join("escape")).unwrap();
    for root in [tree.root(), alias.as_path()] {
        let named = root.join("escape/missing.rs").display().to_string();
        let raw = body("[]", &format!("- `{named}` — exists (2 lines)"));
        let found = inspect_result(&tasks, &project, &raw).unwrap();
        if root == tree.root() {
            assert_eq!(found.len(), 1, "{found:?}");
            assert!(found[0].contains("path leaves the repository"), "{found:?}");
        } else {
            assert!(found.is_empty(), "{found:?}");
        }
    }
}
