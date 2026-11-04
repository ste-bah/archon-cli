use super::*;

fn findings(cwd: &str, command: &str, combined: bool) -> BTreeMap<String, String> {
    findings_at(cwd, command, combined, false)
}

fn findings_at(cwd: &str, command: &str, combined: bool, nested: bool) -> BTreeMap<String, String> {
    let parent = tempfile::tempdir().unwrap();
    let project = parent.path().join("project");
    let repo = if nested { &project } else { parent.path() }.join("repo-dir");
    std::fs::create_dir_all(repo.join("member-dir")).unwrap();
    std::fs::create_dir_all(&project).unwrap();
    for args in [
        vec!["init", "-q"],
        vec!["config", "user.email", "test@example.invalid"],
        vec!["config", "user.name", "test"],
    ] {
        assert!(setup_git(&repo, &args).is_some());
    }
    std::fs::write(
        repo.join("Cargo.toml"),
        "[package]\nname='root-package'\nversion='0.1.0'\n[workspace]\nmembers=['member-dir']\n",
    )
    .unwrap();
    std::fs::write(
        repo.join("member-dir/Cargo.toml"),
        "[package]\nname='member-package'\nversion='0.1.0'\n",
    )
    .unwrap();
    assert!(setup_git(&repo, &["add", "."]).is_some());
    assert!(setup_git(&repo, &["commit", "-qm", "workspace"]).is_some());
    let (mut contract, _) = case("", vec![]);
    contract.acceptance[0].check =
        serde_json::from_value(serde_json::json!({"kind":"command", "cwd":cwd, "command":command}))
            .unwrap();
    super::super::super::super::setup::package_findings(
        &contract, &repo, &project, "HEAD", combined,
    )
}

#[test]
#[allow(non_snake_case)]
fn B2_project_combined_line_three_names_the_root_package() {
    let found = findings(
        "project_root",
        "set -eu\necho ready\ncargo test -p repo-dir --lib new_test -- --exact",
        true,
    );
    assert_eq!(found.len(), 1);
    assert!(
        found["check"]
            .contains("`-p repo-dir` names directory repo-dir; its package is named root-package")
    );
}

#[test]
#[allow(non_snake_case)]
fn B2_project_leading_sibling_cd_keeps_workspace_selection() {
    for combined in [false, true] {
        let found = findings(
            "project_root",
            "cd ../repo-dir && cargo test -p repo-dir",
            combined,
        );
        assert!(found["check"].contains("its package is named root-package"));
    }
}

#[test]
#[allow(non_snake_case)]
fn B2_repo_root_keeps_workspace_selection() {
    for combined in [false, true] {
        for nested in [false, true] {
            assert!(
                findings_at("repo_root", "cargo test -p repo-dir", combined, nested)["check"]
                    .contains("its package is named root-package")
            );
        }
    }
}

#[test]
#[allow(non_snake_case)]
fn B2_project_combined_real_package_has_no_finding() {
    assert!(
        findings(
            "project_root",
            "cargo test -p root-package && cargo build -p member-package",
            true
        )
        .is_empty()
    );
}

#[test]
#[allow(non_snake_case)]
fn B2_project_combined_new_crate_has_no_finding() {
    assert!(findings("project_root", "cargo test -p new-crate", true).is_empty());
}

#[test]
#[allow(non_snake_case)]
fn B2_project_without_a_reachable_workspace_has_no_finding() {
    assert!(findings("project_root", "cargo test -p repo-dir", false).is_empty());
}

#[test]
#[allow(non_snake_case)]
fn B2_project_all_lines_and_segments_report_each_bad_name_once() {
    let found = findings(
        "project_root",
        "set -eu\ncargo test -p repo-dir\ncargo check -p member-dir; cargo build -p repo-dir",
        true,
    );
    assert_eq!(found.len(), 1);
    let text = &found["check"];
    assert_eq!(text.matches("`-p repo-dir`").count(), 1);
    assert_eq!(text.matches("`-p member-dir`").count(), 1);
    assert!(text.contains("its package is named root-package"));
    assert!(text.contains("its package is named member-package"));
}

#[test]
#[allow(non_snake_case)]
fn B2_manifest_path_elsewhere_keeps_the_external_workspace_unknown() {
    for manifest in ["../elsewhere/Cargo.toml", "/elsewhere/Cargo.toml"] {
        assert!(
            findings(
                "project_root",
                &format!("cargo test --manifest-path {manifest} -p repo-dir"),
                true
            )
            .is_empty()
        );
    }
    // A known in-tree manifest still selects its existing workspace.
    assert!(
        findings(
            "project_root",
            "cargo test --manifest-path member-dir/Cargo.toml -p repo-dir",
            true
        )["check"]
            .contains("its package is named root-package")
    );
}

#[test]
#[allow(non_snake_case)]
fn B2_guarded_package_failures_have_no_static_finding() {
    for command in [
        "cargo test -p repo-dir || cargo test -p root-package",
        "! cargo test -p repo-dir",
        "if cargo test -p repo-dir; then echo ok; fi",
        "true && cargo test -p repo-dir || true",
        "cargo check -p member-dir || true",
    ] {
        assert!(
            findings("project_root", command, true).is_empty(),
            "{command}"
        );
    }
}

#[test]
#[allow(non_snake_case)]
fn B2_heredoc_package_text_is_data() {
    for redirect in ["<<EOF", "<<'EOF'", "<<\"EOF\"", "<<-EOF"] {
        let command = format!(
            "set -eu\ncat > notes.txt {redirect}\ncargo test -p repo-dir\nEOF\ngrep -q repo-dir notes.txt"
        );
        assert!(findings("project_root", &command, true).is_empty());
    }
}

#[test]
#[allow(non_snake_case)]
fn B2_real_command_after_heredoc_still_names_the_root_package() {
    let found = findings(
        "project_root",
        "cat <<'EOF'\ncargo test -p member-dir || true\nEOF\ncargo test -p repo-dir",
        true,
    );
    assert_eq!(found.len(), 1);
    assert!(found["check"].contains("its package is named root-package"));
    assert!(!found["check"].contains("member-dir"));
}

#[test]
#[allow(non_snake_case)]
fn B2_workspace_selecting_subcommands_report_directory_names() {
    for subcommand in [
        "test", "build", "check", "run", "bench", "doc", "clippy", "rustc", "rustdoc", "tree",
        "clean",
    ] {
        let found = findings(
            "project_root",
            &format!("cargo {subcommand} -p repo-dir"),
            true,
        );
        assert!(
            found["check"].contains("its package is named root-package"),
            "{subcommand}"
        );
    }
}

#[test]
#[allow(non_snake_case)]
fn B2_update_dependency_selector_has_no_finding() {
    assert!(findings("project_root", "cargo update -p repo-dir", true).is_empty());
}
