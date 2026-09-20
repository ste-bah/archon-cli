use super::*;
use std::path::PathBuf;
use std::process::Command;

fn git(repo: &Path, args: &[&str]) {
    let output = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(args)
        .output()
        .expect("git starts");
    assert!(
        output.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

/// A repository with `src/lib.rs` and `src/existing.rs` committed, and a
/// task root beside it that records the repository.
fn grounded() -> (tempfile::TempDir, PathBuf, PathBuf, RepositoryTree) {
    let temp = tempfile::tempdir().unwrap();
    let repo = temp.path().join("repo");
    std::fs::create_dir_all(repo.join("src")).unwrap();
    std::fs::write(repo.join("src/lib.rs"), "pub mod existing;\n").unwrap();
    std::fs::write(repo.join("src/existing.rs"), "pub fn f() {}\n").unwrap();
    git(&repo, &["init", "-q"]);
    git(&repo, &["config", "user.email", "t@example.invalid"]);
    git(&repo, &["config", "user.name", "t"]);
    git(&repo, &["add", "."]);
    git(&repo, &["commit", "-q", "-m", "initial"]);
    let project = temp.path().join("project");
    let tasks = project.join("tasks/PRD-X");
    std::fs::create_dir_all(&tasks).unwrap();
    let record = archon_workflow::repository_record::RepositoryRecordV1 {
        schema_version: 1,
        repository_root: repo.canonicalize().unwrap().display().to_string(),
        base_commit: archon_workflow::repository_record::git_head(&repo).unwrap(),
        decomposition_run_id: "wf-test".into(),
        recorded_at: "now".into(),
    };
    archon_workflow::repository_record::write_repository_record(&tasks, &record).unwrap();
    let tree = RepositoryTree::load(&record).unwrap();
    (temp, project, tasks, tree)
}

fn claims(text: &str) -> Vec<(String, Claim)> {
    extract_claims(text)
        .into_iter()
        .map(|claim| (claim.path, claim.claim))
        .collect()
}

#[test]
fn unambiguous_existence_sentences_are_claims() {
    assert_eq!(
        claims("`src/existing.rs` does not exist yet."),
        vec![("src/existing.rs".to_string(), Claim::Absent)]
    );
    assert_eq!(
        claims("- `src/new_module.rs` — absent"),
        vec![("src/new_module.rs".to_string(), Claim::Absent)]
    );
    assert_eq!(
        claims("The file `src/lib.rs` already exists (12 lines)."),
        vec![("src/lib.rs".to_string(), Claim::Exists)]
    );
    assert_eq!(
        claims("`crates/x/src/lib.rs`: exists (40 lines)"),
        vec![("crates/x/src/lib.rs".to_string(), Claim::Exists)]
    );
    assert_eq!(
        claims("Modify `src/lib.rs` to export the module."),
        vec![("src/lib.rs".to_string(), Claim::Exists)]
    );
    assert_eq!(
        claims("Create a new file `src/widgets.rs` holding the type."),
        vec![("src/widgets.rs".to_string(), Claim::Absent)]
    );
    assert_eq!(
        claims("`src/widgets/` (new) will hold the modules."),
        vec![("src/widgets".to_string(), Claim::Absent)]
    );
    assert_eq!(
        claims("`/abs/repo/src/lib.rs` exists (3 lines)"),
        vec![("/abs/repo/src/lib.rs".to_string(), Claim::Exists)]
    );
}

#[test]
fn prose_that_is_not_about_the_path_itself_is_not_a_claim() {
    for text in [
        "`src/lib.rs` is missing the trait impl.",
        "The feature does not exist in `src/lib.rs`.",
        "Create `src/lib.rs` if needed.",
        "`src/lib.rs` new module for widgets",
        "Run `cargo test -p x` and read `src/lib.rs` for context.",
        "`src/*.rs` do not exist",
        "```\n`src/lib.rs` does not exist\n```",
        "`README` does not exist",
        "`https://example.com/a.md` does not exist",
        "`../elsewhere/x.rs` exists",
        "`src/lib.rs` exists only to re-export",
    ] {
        assert!(
            claims(text).is_empty(),
            "{text:?} yielded {:?}",
            claims(text)
        );
    }
}

#[test]
fn a_false_does_not_exist_is_a_blocking_finding_naming_path_and_truth() {
    let (_temp, project, _tasks, tree) = grounded();
    let body = "## Plan\n\n`src/existing.rs` does not exist yet; this task creates it.\n";
    let findings = findings_against(&tree, &project, "TASK-X-001", body);
    assert_eq!(findings.len(), 1, "{findings:?}");
    let finding = &findings[0];
    assert!(
        finding.starts_with("TASK-X-001: the body says `src/existing.rs` does not exist"),
        "{finding}"
    );
    assert!(
        finding.contains(&format!("at base commit {}", tree.base_commit())),
        "{finding}"
    );
    assert!(finding.contains("it exists in repository"), "{finding}");
}

#[test]
fn a_false_exists_is_a_blocking_finding_and_a_true_claim_is_not() {
    let (_temp, project, _tasks, tree) = grounded();
    let body = "Modify `src/ghost.rs` to add the field.\n`src/lib.rs` exists (1 line) and `src/new.rs` does not exist.\n";
    let findings = findings_against(&tree, &project, "TASK-X-002", body);
    assert_eq!(findings.len(), 1, "{findings:?}");
    assert!(
        findings[0].contains("`src/ghost.rs` exists"),
        "{}",
        findings[0]
    );
    assert!(
        findings[0].contains("it is absent from repository"),
        "{}",
        findings[0]
    );
}

#[test]
fn drift_since_the_base_refutes_nothing() {
    let (_temp, project, _tasks, tree) = grounded();
    // Added after the base: absent at base, present in the checkout.
    std::fs::write(tree.root().join("src/later.rs"), "").unwrap();
    // Deleted after the base: present at base, absent from the checkout.
    std::fs::remove_file(tree.root().join("src/existing.rs")).unwrap();
    let body = "`src/later.rs` does not exist. `src/existing.rs` exists (1 line). `src/later.rs` exists. `src/existing.rs` does not exist.";
    assert!(findings_against(&tree, &project, "TASK-X-003", body).is_empty());
}

#[test]
fn an_exists_claim_about_a_project_file_is_about_the_project_not_the_repository() {
    let (_temp, project, _tasks, tree) = grounded();
    std::fs::create_dir_all(project.join("artifacts")).unwrap();
    std::fs::write(project.join("artifacts/report.json"), "{}").unwrap();
    let body = "`artifacts/report.json` exists and is regenerated.";
    assert!(findings_against(&tree, &project, "TASK-X-004", body).is_empty());
}

#[test]
fn a_task_set_without_a_record_is_not_checked_and_the_set_gate_names_each_task() {
    let temp = tempfile::tempdir().unwrap();
    let tasks = temp.path().join("tasks");
    std::fs::create_dir_all(&tasks).unwrap();
    let file = tasks.join("TASK-X-001.md");
    assert!(
        inspect(
            temp.path(),
            &tasks,
            "TASK-X-001",
            &file,
            "`src/lib.rs` does not exist"
        )
        .unwrap()
        .is_empty()
    );

    let (_temp, project, tasks, _tree) = grounded();
    std::fs::write(
        tasks.join("TASK-X-001.md"),
        "# TASK-X-001\n\n```yaml\ntask_id: TASK-X-001\ntitle: T\ncomplexity: low\nstatus: ready\ndepends_on: []\nblocks: []\nimplements: [\"AC-X-001\"]\nrequired_env_keys: []\nrequired_tools: []\ndeliverable_contracts: []\n```\n\n## Plan\n\n`src/existing.rs` does not exist.\n\n## Focused Tests\n\n- `cargo test -p x`\n",
    )
    .unwrap();
    let findings = set_findings(&project, &tasks).unwrap();
    assert_eq!(
        findings.len(),
        1,
        "{:?}",
        findings.iter().map(|f| &f.text).collect::<Vec<_>>()
    );
    assert_eq!(findings[0].subject, "TASK-X-001");
    assert_eq!(
        findings[0].remediation_scope,
        archon_workflow::RemediationScope::Body
    );
    assert_eq!(
        findings[0].source_path.as_deref(),
        Some(tasks.join("TASK-X-001.md").as_path())
    );
}
