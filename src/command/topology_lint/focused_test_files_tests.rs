//! Batch O2 (PLAN-9): the files a focused test runs, and the set finding
//! for one no task declares.

use super::*;
use archon_workflow::repository_record::{RepositoryRecordV1, git_head, write_repository_record};

fn paths(list: &[&str]) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    for path in list {
        let mut at = String::new();
        for segment in path.split('/') {
            if !at.is_empty() {
                at.push('/');
            }
            at.push_str(segment);
            out.insert(at.clone());
        }
    }
    out
}

fn packages() -> BTreeMap<String, String> {
    [("engine", "crates/engine"), ("cli", "")]
        .into_iter()
        .map(|(name, dir)| (name.to_string(), dir.to_string()))
        .collect()
}

fn files(command: &str, tree: &BTreeSet<String>) -> Vec<Vec<String>> {
    required_files(command, tree, &packages())
        .into_iter()
        .map(|required| required.alternatives)
        .collect()
}

#[test]
fn the_files_a_command_runs_are_read_from_it_for_any_runner() {
    let tree = paths(&[
        "crates/engine/tests/gates.rs",
        "crates/engine/src/store_tests.rs",
        "crates/engine/src/store.rs",
        "crates/engine/src/lib.rs",
        "tests/test_api.py",
        "scripts/check.sh",
    ]);
    assert_eq!(
        files("cargo test -p engine --test gates", &tree),
        [["crates/engine/tests/gates.rs"]]
    );
    assert_eq!(
        files(
            "cargo nextest run --package=engine --test=gates -- --exact",
            &tree
        ),
        [["crates/engine/tests/gates.rs"]]
    );
    // A target the task has yet to create: either shape it may take.
    assert_eq!(
        files("cargo test -p engine --test fresh", &tree),
        [[
            "crates/engine/tests/fresh.rs",
            "crates/engine/tests/fresh/main.rs"
        ]]
    );
    assert_eq!(
        files("cargo test -p engine store::tests", &tree),
        [["crates/engine/src/store_tests.rs"]]
    );
    // A crate root names nothing (as at run time).
    assert!(files("cargo test -p engine lib", &tree).is_empty());
    assert_eq!(
        files("python -m pytest tests/test_api.py::test_get -q", &tree),
        [["tests/test_api.py"]]
    );
    assert_eq!(
        files("bash scripts/check.sh", &tree),
        [["scripts/check.sh"]]
    );
    assert!(files("npm test", &tree).is_empty());
    // m4: an option's value is no file the command runs, even one that
    // exists; a package no manifest declares is named as unresolvable.
    let with_manifest = {
        let mut tree = tree.clone();
        tree.extend(paths(&["crates/engine/Cargo.toml"]));
        tree
    };
    assert_eq!(
        files(
            "cargo test --manifest-path crates/engine/Cargo.toml -p engine --test gates",
            &with_manifest
        ),
        [["crates/engine/tests/gates.rs"]]
    );
    assert_eq!(
        unknown_packages("cargo test -p ghost --test gates", &packages()),
        ["ghost"]
    );
    assert!(unknown_packages("cargo test --package=engine", &packages()).is_empty());
    assert_eq!(
        package_name("[package]\nname = \"engine\"\n").as_deref(),
        Some("engine")
    );
    assert_eq!(package_name("[workspace]\nmembers = []\n"), None);
}

fn git(repo: &Path, args: &[&str]) {
    let out = std::process::Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(args)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
}

fn task(tasks: &Path, id: &str, owns: &str, focused: &str) {
    std::fs::write(
        tasks.join(format!("{id}.md")),
        format!(
            "# {id}\n\n```yaml\ntask_id: {id}\ntitle: T\ncomplexity: low\nstatus: ready\ndepends_on: []\nblocks: []\nimplements: [\"AC-X-001\"]\nrequired_env_keys: []\nrequired_tools: []\ndeliverable_contracts: []\n```\n\n## Files Expected to Change\n\n- `{owns}` — exists\n\n## Focused Tests\n\n- `{focused}`\n"
        ),
    )
    .unwrap();
}

#[test]
fn a_focused_test_file_no_task_declares_is_a_body_finding_of_the_declaring_task() {
    let temp = tempfile::tempdir().unwrap();
    let repo = temp.path().join("repo");
    for (file, text) in [
        ("Cargo.toml", "[workspace]\nmembers = [\"crates/engine\"]\n"),
        ("crates/engine/Cargo.toml", "[package]\nname = \"engine\"\n"),
        ("crates/engine/src/lib.rs", "pub fn f() {}\n"),
        ("crates/engine/tests/gates.rs", "#[test] fn t() {}\n"),
        ("crates/engine/tests/owned.rs", "#[test] fn t() {}\n"),
    ] {
        let path = repo.join(file);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }
    git(&repo, &["init", "-q"]);
    git(&repo, &["config", "user.email", "t@example.invalid"]);
    git(&repo, &["config", "user.name", "t"]);
    git(&repo, &["add", "."]);
    git(&repo, &["commit", "-q", "-m", "initial"]);
    let tasks = temp.path().join("project/tasks/PRD-X");
    std::fs::create_dir_all(&tasks).unwrap();
    write_repository_record(
        &tasks,
        &RepositoryRecordV1 {
            schema_version: 1,
            repository_root: repo.canonicalize().unwrap().display().to_string(),
            base_commit: git_head(&repo).unwrap(),
            decomposition_run_id: "wf-test".into(),
            recorded_at: "now".into(),
        },
    )
    .unwrap();
    task(
        &tasks,
        "TASK-X-001",
        "crates/engine/src/lib.rs",
        "cargo test -p engine --test gates",
    );
    task(
        &tasks,
        "TASK-X-002",
        "crates/engine/tests/owned.rs",
        "cargo test -p engine --test owned",
    );
    let findings = set_findings(&tasks).unwrap();
    assert_eq!(findings.len(), 1, "{findings:?}");
    let finding = &findings[0];
    assert!(
        finding.text.starts_with("task TASK-X-001:")
            && finding.text.contains("crates/engine/tests/gates.rs"),
        "{finding:?}"
    );
    assert_eq!(finding.subject, "TASK-X-001");
    assert_eq!(
        finding.remediation_scope,
        archon_workflow::RemediationScope::Body
    );
    assert!(
        format!("{finding:?}").contains("TASK-X-001.md"),
        "{finding:?}"
    );
    // Declared, it is no finding.
    task(
        &tasks,
        "TASK-X-003",
        "crates/engine/tests/gates.rs",
        "cargo test -p engine --test owned",
    );
    assert!(set_findings(&tasks).unwrap().is_empty());
    // m4: fails closed -- a task file it cannot parse is a finding, never a
    // task whose tests are silently unchecked.
    std::fs::write(tasks.join("TASK-X-004.md"), "# TASK-X-004\n\nno metadata\n").unwrap();
    let findings = set_findings(&tasks).unwrap();
    assert_eq!(findings.len(), 1, "{findings:?}");
    assert!(
        findings[0].text.contains("cannot be parsed"),
        "{findings:?}"
    );
}
