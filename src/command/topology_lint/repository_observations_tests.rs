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

/// A repository with `src/lib.rs` (1 line), `src/existing.rs` (3 lines) and
/// the directory `src/widgets/` committed, recorded beside a task root.
fn grounded() -> (tempfile::TempDir, PathBuf, PathBuf, RepositoryTree) {
    let temp = tempfile::tempdir().unwrap();
    let repo = temp.path().join("repo");
    std::fs::create_dir_all(repo.join("src/widgets")).unwrap();
    std::fs::write(repo.join("src/lib.rs"), "pub mod existing;\n").unwrap();
    std::fs::write(repo.join("src/existing.rs"), "pub fn f() {}\n\nfn g() {}").unwrap();
    std::fs::write(repo.join("src/widgets/mod.rs"), "").unwrap();
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

fn body(contracts: &str, files_section: &str) -> String {
    format!(
        "# TASK-X-001\n\n```yaml\ntask_id: TASK-X-001\ntitle: T\ncomplexity: low\nstatus: ready\ndepends_on: []\nblocks: []\nimplements: [\"AC-X-001\"]\nrequired_env_keys: []\nrequired_tools: []\ndeliverable_contracts: {contracts}\n```\n\n## Plan\n\nDo the thing.\n\n## Files Expected to Change\n\n{files_section}\n\n## Focused Tests\n\n- `cargo test -p x`\n"
    )
}

fn findings(tasks: &Path, project: &Path, tree: &RepositoryTree, raw: &str) -> Vec<String> {
    let path = tasks.join("TASK-X-001.md");
    let task = archon_workflow::task_universe::parsing::parse_task_file(&path, raw).unwrap();
    findings_against(tree, project, "TASK-X-001", raw, &task)
        .into_iter()
        .map(|(_, text)| text)
        .collect()
}

#[test]
fn the_grammar_parses_exactly_three_observations() {
    assert_eq!(
        parse_observation(" — exists (12 lines)"),
        Some(Observation::File { lines: 12 })
    );
    assert_eq!(
        parse_observation(": `exists (1 line)`"),
        Some(Observation::File { lines: 1 })
    );
    assert_eq!(
        parse_observation(" - exists (1,204 lines): add the field"),
        Some(Observation::File { lines: 1204 })
    );
    assert_eq!(
        parse_observation(" — exists (directory)"),
        Some(Observation::Directory)
    );
    assert_eq!(
        parse_observation("** — exists (7 lines)"),
        Some(Observation::File { lines: 7 }),
        "emphasis closes before the observation"
    );
    assert_eq!(
        parse_observation(") exists (7 lines)"),
        Some(Observation::File { lines: 7 })
    );
    assert_eq!(
        parse_observation(" (exists (dir))"),
        None,
        "a parenthesis is not a separator"
    );
    assert_eq!(parse_observation(" — absent"), Some(Observation::Absent));
    assert_eq!(
        parse_observation(" — absent; this task creates it"),
        Some(Observation::Absent)
    );
    for text in [
        " — exists (N lines)",
        " — exists",
        " — exists (about 12 lines)",
        " — absently",
        " — not observed from this authoring run",
        " — present (12 lines)",
        " will be created",
        "",
    ] {
        assert_eq!(parse_observation(text), None, "{text:?}");
    }
    assert_eq!(count_lines(b""), 0);
    assert_eq!(count_lines(b"a\n"), 1);
    assert_eq!(count_lines(b"a\nb"), 2);
    assert_eq!(count_lines(b"pub fn f() {}\n\nfn g() {}"), 3);
}

#[test]
fn a_deliverable_with_no_observation_is_a_blocking_finding_naming_the_path() {
    let (_temp, project, tasks, tree) = grounded();
    let raw = body(
        "[]",
        "- `src/lib.rs`: add the module declaration\n- `src/new.rs`",
    );
    let found = findings(&tasks, &project, &tree, &raw);
    assert_eq!(found.len(), 2, "{found:?}");
    assert!(
        found[0]
            .starts_with("TASK-X-001: deliverable path `src/lib.rs` has no verifiable observation"),
        "{}",
        found[0]
    );
    assert!(found[0].contains("exists (N lines)"), "{}", found[0]);
    assert!(
        found[1]
            .starts_with("TASK-X-001: deliverable path `src/new.rs` has no verifiable observation"),
        "{}",
        found[1]
    );
}

#[test]
fn unobserved_wording_is_a_blocking_finding_even_with_the_literal_grammar_in_it() {
    let (_temp, project, tasks, tree) = grounded();
    let repo = tree.root().display();
    let raw = body(
        "[]",
        &format!(
            "- `{repo}/src/lib.rs` — not observed from this authoring run (repository root outside this run's allowed tool directories) — implementer must record exists (N lines) or absent"
        ),
    );
    let found = findings(&tasks, &project, &tree, &raw);
    assert_eq!(found.len(), 1, "{found:?}");
    assert!(
        found[0]
            .starts_with("TASK-X-001: deliverable path `src/lib.rs` is described as unobserved"),
        "{}",
        found[0]
    );
    assert!(
        found[0].contains("not observed from this authoring run"),
        "{}",
        found[0]
    );
}

#[test]
fn a_wrong_line_count_is_a_blocking_finding_carrying_the_true_count() {
    let (_temp, project, tasks, tree) = grounded();
    let raw = body(
        "[]",
        "- `src/existing.rs` — exists (2 lines): add g\n- `src/lib.rs` — exists (1 line)",
    );
    let found = findings(&tasks, &project, &tree, &raw);
    assert_eq!(found.len(), 1, "{found:?}");
    assert!(
        found[0].contains("`src/existing.rs` exists (2 lines) but the checkout at"),
        "{}",
        found[0]
    );
    assert!(
        found[0].contains("has 3 lines; rewrite the observation as exists (3 lines)"),
        "{}",
        found[0]
    );
}

#[test]
fn correct_observations_in_either_spelling_pass_and_wrong_existence_does_not() {
    let (_temp, project, tasks, tree) = grounded();
    let repo = tree.root().display();
    let raw = body(
        "[]",
        &format!(
            "- `{repo}/src/existing.rs` — `exists (3 lines)`: add g\n- `src/lib.rs` — exists (1 line)\n- `src/widgets/` — exists (directory)\n- `src/new.rs` — absent; this task creates it"
        ),
    );
    assert!(findings(&tasks, &project, &tree, &raw).is_empty());

    let raw = body(
        "[]",
        "- `src/new.rs` — exists (4 lines)\n- `src/existing.rs` — absent\n- `src/widgets/` — exists (2 lines)",
    );
    let found = findings(&tasks, &project, &tree, &raw);
    assert_eq!(found.len(), 3, "{found:?}");
    assert!(
        found[0].contains("`src/existing.rs` is absent but it exists in repository"),
        "{}",
        found[0]
    );
    assert!(
        found[0].contains("(3 lines); rewrite the observation as exists (3 lines)"),
        "{}",
        found[0]
    );
    assert!(
        found[1].contains("`src/new.rs` exists but it is absent from repository"),
        "{}",
        found[1]
    );
    assert!(
        found[2].contains("`src/widgets` exists (2 lines) but it is a directory"),
        "{}",
        found[2]
    );
}

#[test]
fn the_observation_may_live_in_another_section_and_contracts_count_as_deliverables() {
    let (_temp, project, tasks, tree) = grounded();
    let mut raw = body(
        "[{kind: source, artifact_path: src/existing.rs}, {kind: registry, artifact_path: artifacts/registry.json}]",
        "- `src/lib.rs`",
    );
    raw.push_str("\n## Repository Observations\n\n- `src/lib.rs` — exists (1 line)\n- `src/existing.rs` — exists (3 lines)\n- `artifacts/registry.json` — absent\n");
    assert!(findings(&tasks, &project, &tree, &raw).is_empty());
    // A project artifact that already exists outside the repository needs no
    // repository observation.
    std::fs::create_dir_all(project.join("artifacts")).unwrap();
    std::fs::write(project.join("artifacts/registry.json"), "{}").unwrap();
    let raw = body(
        "[{kind: registry, artifact_path: artifacts/registry.json}]",
        "- `src/lib.rs` — exists (1 line)",
    );
    assert!(findings(&tasks, &project, &tree, &raw).is_empty());
}

#[test]
fn the_body_lint_and_set_gate_report_each_path_once() {
    let (_temp, project, tasks, tree) = grounded();
    // "absent" is also a claim `repository_claims` would refute; the
    // observation finding, which carries the line count, is the one kept.
    let raw = body("[]", "- `src/existing.rs` — absent");
    let path = tasks.join("TASK-X-001.md");
    let found =
        super::super::repository_claims::body_findings(&tree, &project, "TASK-X-001", &path, &raw);
    assert_eq!(found.len(), 1, "{found:?}");
    assert!(
        found[0].contains("rewrite the observation as exists (3 lines)"),
        "{}",
        found[0]
    );

    std::fs::write(&path, &raw).unwrap();
    let set = super::super::repository_claims::set_findings(&project, &tasks).unwrap();
    assert_eq!(
        set.len(),
        1,
        "{:?}",
        set.iter().map(|f| &f.text).collect::<Vec<_>>()
    );
    assert_eq!(set[0].subject, "TASK-X-001");
    assert_eq!(
        set[0].remediation_scope,
        archon_workflow::RemediationScope::Body
    );
    assert!(
        set[0]
            .text
            .contains("`src/existing.rs` is absent but it exists"),
        "{}",
        set[0].text
    );
}
