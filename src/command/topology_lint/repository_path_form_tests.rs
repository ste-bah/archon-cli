use super::*;

#[test]
fn live_shaped_suffixes_have_distinct_body_findings_in_both_checks() {
    for count in [1, 2, 4] {
        let (_temp, project, tasks, tree) = recorded("PRD-X", |repo| {
            for index in 0..count {
                let dir = repo.join(format!("crates/provider{index}/src/providers"));
                std::fs::create_dir_all(dir.join("capability_tests")).unwrap();
                std::fs::create_dir_all(dir.join("capability")).unwrap();
                for path in [
                    "capability_tests/engine_tests.rs",
                    "capability/mod.rs",
                    "mod.rs",
                ] {
                    std::fs::write(dir.join(path), "// test\n").unwrap();
                }
            }
        });
        for path in [
            "capability_tests/engine_tests.rs",
            "capability/",
            "providers/mod.rs",
        ] {
            for predicate in ["exists.", "does not exist.", "— exists (1 line)"] {
                let raw = body("[]", "") + &format!("\n## Notes\n`{path}` {predicate}\n");
                let found = super::super::super::repository_claims::body_findings(
                    &tree,
                    &project,
                    "TASK-X-001",
                    &tasks.join("TASK-X-001.md"),
                    &raw,
                )
                .unwrap();
                assert_eq!(found.len(), 1, "{raw}: {found:?}");
                assert!(
                    found[0].contains("[repository_path_not_root_relative]"),
                    "{found:?}"
                );
                assert!(found[0].contains("is not repository-root relative: no such path from the repository root; write the full repository-root path (matches:"));
                assert!(!found[0].contains("absent from repository"));
                assert_eq!(found[0].matches("`crates/").count(), count.min(3));
            }
        }
        let raw = body("[]", "- `providers/mod.rs` — exists (1 line)");
        std::fs::write(tasks.join("TASK-X-001.md"), raw).unwrap();
        let set = super::super::super::repository_claims::set_findings(&project, &tasks).unwrap();
        assert_eq!(set.len(), 1);
        assert_eq!(
            set[0].remediation_scope,
            archon_workflow::RemediationScope::Body
        );
        let expected = archon_workflow::defect::DeterministicDefect::new(
            "repository_path_not_root_relative",
            "TASK-X-001",
            "claims/0",
        );
        assert_eq!(set[0].deterministic_defect.as_ref(), Some(&expected));
    }
}

#[test]
fn headings_keep_missing_slashed_and_absolute_claims() {
    let (_temp, project, tasks, tree) = grounded();
    for named in [
        "unknown/ghost.rs".to_string(),
        tree.root().join("unknown/ghost.rs").display().to_string(),
    ] {
        for predicate in ["exists.", "— exists (1 line)"] {
            let raw = body("[]", "") + &format!("\n## In `src/`\n`{named}` {predicate}\n");
            let found = super::super::super::repository_claims::body_findings(
                &tree,
                &project,
                "TASK-X-001",
                &tasks.join("TASK-X-001.md"),
                &raw,
            )
            .unwrap();
            let expected = if named == "unknown/ghost.rs" && predicate.starts_with('—') {
                2
            } else {
                1
            };
            assert_eq!(found.len(), expected, "{raw}: {found:?}");
            assert!(
                found
                    .iter()
                    .all(|finding| finding.contains("absent from repository")),
                "{found:?}"
            );
            assert!(found[0].contains("`unknown/ghost.rs`"), "{found:?}");
            assert!(!found[0].contains("not repository-root relative"));
        }
    }
    let claim = super::super::super::repository_claims::extract_claims(
        "## In `src/`\n`unknown/ghost.rs` exists.",
    )
    .pop()
    .unwrap();
    let paths =
        super::super::super::repository_claims::resolve_scoped_paths(&tree, &claim).unwrap();
    assert!(paths.contains(&"unknown/ghost.rs".to_string()), "{paths:?}");
    assert!(
        paths.contains(&"src/unknown/ghost.rs".to_string()),
        "{paths:?}"
    );
}

#[test]
fn outside_absolute_descendants_of_files_are_skipped_in_all_entry_points() {
    let (_temp, project, tasks, tree) = grounded();
    let outside = project.join("outside-file");
    std::fs::write(&outside, "file\n").unwrap();
    let named = outside.join("nested/child.rs").display().to_string();
    assert!(
        super::super::super::repository_paths::relative_path(&tree, &named)
            .unwrap()
            .is_none()
    );
    for predicate in ["", "exists.", "— exists (1 line)"] {
        let raw = body("[]", &format!("- `{named}` {predicate}"))
            + &format!("\n## In `src/`\n`{named}` {predicate}\n");
        let found = super::super::super::repository_claims::body_findings(
            &tree,
            &project,
            "TASK-X-001",
            &tasks.join("TASK-X-001.md"),
            &raw,
        )
        .unwrap();
        assert!(found.is_empty(), "{raw}: {found:?}");
    }
}
