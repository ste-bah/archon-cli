use super::*;

fn contract() -> AcceptanceContract {
    serde_json::from_value(serde_json::json!({
        "schema_version": 1, "prd": {"path":"p.md", "digest":"d"},
        "gap_policy": {"permitted_acceptance_ids":[], "forbidden_phrases":[], "required_fields":[]},
        "acceptance": [
            {"id":"PROJECT", "criterion":"project sibling", "check":{"kind":"command", "command":"cat ../sibling/Cargo.toml", "cwd":"project_root"}, "judgment":{"verdict":"accepted", "counterexample":"", "reason":"", "host_call_id":"test"}},
            {"id":"REPO", "criterion":"repo sibling", "check":{"kind":"command", "command":"cat ../sibling/Cargo.toml", "cwd":"repo_root"}, "judgment":{"verdict":"accepted", "counterexample":"", "reason":"", "host_call_id":"test"}}
        ],
        "supplementary": []
    })).unwrap()
}

fn git_sibling(path: &Path, name: &str) -> String {
    std::fs::create_dir_all(path).unwrap();
    std::fs::write(
        path.join("Cargo.toml"),
        format!("[package]\nname='{name}'\nversion='0.1.0'\nedition='2021'\n"),
    )
    .unwrap();
    let run = |args: &[&str]| {
        let output = std::process::Command::new("git")
            .arg("-C")
            .arg(path)
            .args(args)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8_lossy(&output.stdout).trim().to_string()
    };
    run(&["init", "-q"]);
    run(&["config", "user.email", "test@example.invalid"]);
    run(&["config", "user.name", "test"]);
    run(&["add", "Cargo.toml"]);
    run(&["commit", "-q", "-m", name]);
    run(&["rev-parse", "HEAD"])
}

fn copied_sibling_revisions(
    shared: bool,
) -> (
    Vec<SiblingRevision>,
    BTreeMap<String, Vec<SiblingRevision>>,
    String,
    String,
) {
    let source = tempfile::tempdir().unwrap();
    let destination = tempfile::tempdir().unwrap();
    let project_parent = source.path().join("project-area");
    let repository_parent = if shared {
        project_parent.clone()
    } else {
        source.path().join("repository-area")
    };
    let project = project_parent.join("project");
    let repository = repository_parent.join("repository");
    std::fs::create_dir_all(&project).unwrap();
    std::fs::create_dir_all(&repository).unwrap();
    let project_head = git_sibling(&project_parent.join("sibling"), "project-sibling");
    let repository_head = if shared {
        project_head.clone()
    } else {
        git_sibling(&repository_parent.join("sibling"), "repository-sibling")
    };
    let copied_project = destination.path().join("project-area/project");
    let copied_repository = destination.path().join("repository-area/repository");
    std::fs::create_dir_all(&copied_project).unwrap();
    std::fs::create_dir_all(&copied_repository).unwrap();
    let (siblings, checks) = super::super::siblings::copy_for_checks_with_check_map(
        &copied_project,
        &project,
        &copied_repository,
        &repository,
        &contract(),
    )
    .unwrap();
    (siblings, checks, project_head, repository_head)
}

#[test]
fn same_relative_sibling_path_keeps_distinct_git_revisions_per_root() {
    let (siblings, checks, project_head, repository_head) = copied_sibling_revisions(false);
    assert_ne!(project_head, repository_head);
    assert_eq!(siblings.len(), 2);
    assert_eq!(
        checks["PROJECT"][0].commit.as_deref(),
        Some(project_head.as_str())
    );
    assert_eq!(
        checks["REPO"][0].commit.as_deref(),
        Some(repository_head.as_str())
    );
}

#[test]
fn same_real_sibling_reached_from_both_roots_has_one_revision_entry() {
    let (siblings, checks, project_head, repository_head) = copied_sibling_revisions(true);
    assert_eq!(project_head, repository_head);
    assert_eq!(siblings.len(), 1);
    assert_eq!(checks["PROJECT"].len(), 1);
    assert_eq!(checks["REPO"].len(), 1);
    assert_eq!(checks["PROJECT"][0].path, checks["REPO"][0].path);
    assert_eq!(
        checks["PROJECT"][0].commit.as_deref(),
        Some(project_head.as_str())
    );
}
