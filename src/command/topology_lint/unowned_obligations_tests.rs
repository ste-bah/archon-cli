use super::*;
use archon_workflow::repository_record::{RepositoryRecordV1, git_head, write_repository_record};
use std::path::PathBuf;
use std::process::Command;

fn git(repo: &Path, args: &[&str]) {
    let output = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(args)
        .output()
        .expect("git starts");
    assert!(output.status.success(), "git {args:?}");
}

/// A recorded repository with an owned lane, an unowned twin lane and a
/// file two directories share a name with.
fn grounded() -> (tempfile::TempDir, PathBuf) {
    let temp = tempfile::tempdir().unwrap();
    let repo = temp.path().join("repo");
    for path in [
        "crates/t/src/data_store/ingest.rs",
        "crates/t/src/providers/store.rs",
        "crates/t/src/a/mod.rs",
        "crates/t/src/b/mod.rs",
    ] {
        let target = repo.join(path);
        std::fs::create_dir_all(target.parent().unwrap()).unwrap();
        std::fs::write(target, "//\n").unwrap();
    }
    git(&repo, &["init", "-q"]);
    git(&repo, &["config", "user.email", "t@example.invalid"]);
    git(&repo, &["config", "user.name", "t"]);
    git(&repo, &["add", "."]);
    git(&repo, &["commit", "-q", "-m", "initial"]);
    let tasks = temp.path().join("project/tasks/PRD-T");
    std::fs::create_dir_all(&tasks).unwrap();
    let record = RepositoryRecordV1 {
        schema_version: 1,
        repository_root: repo.canonicalize().unwrap().display().to_string(),
        base_commit: git_head(&repo).unwrap(),
        decomposition_run_id: "wf-test".into(),
        recorded_at: "now".into(),
    };
    write_repository_record(&tasks, &record).unwrap();
    (temp, tasks)
}

fn task(body: &str) -> String {
    format!(
        "# TASK-T-001\n\n```yaml\ntask_id: TASK-T-001\ntitle: T\ncomplexity: low\nstatus: ready\ndepends_on: []\nblocks: []\nimplements: [\"AC-T-001\"]\nrequired_env_keys: []\nrequired_tools: []\ndeliverable_contracts: []\n```\n\n## Files Expected to Change\n\n- `crates/t/src/data_store/ingest.rs` — exists\n\n## Files Forbidden to Change\n\n- `crates/t/src/providers/store.rs` must not be edited here\n\n{body}\n## Focused Tests\n\n- `cargo test -p t`\n"
    )
}

#[test]
fn an_obligation_on_an_undeclared_file_warns_by_name() {
    let (_temp, tasks) = grounded();
    std::fs::write(
        tasks.join("TASK-T-001.md"),
        task(
            "## Risks\n\n- `data_store/ingest.rs` and `providers/store.rs` both write versions;\n  they must stay consistent or the registry diverges.\n- Supporting surfaces observed, not deliverables: `crates/t/src/a/mod.rs`.\n- `mod.rs` must stay small.\n",
        ),
    )
    .unwrap();
    let found = warnings(&tasks).unwrap().unwrap();
    assert_eq!(found.len(), 1, "{found:#?}");
    assert!(
        found[0].starts_with("TASK-T-001 obliges `crates/t/src/providers/store.rs`"),
        "a unique suffix resolves; an observed mention and an ambiguous basename do not: {}",
        found[0]
    );
    let report = section(Some(&tasks));
    assert!(report.contains("## unowned obligations"), "{report}");
    assert!(report.contains("WARNING: TASK-T-001 obliges"), "{report}");
}

#[test]
fn a_declared_file_a_negation_or_an_ownership_section_does_not_warn() {
    let (_temp, tasks) = grounded();
    std::fs::write(
        tasks.join("TASK-T-001.md"),
        task(
            "## Notes\n\nThe lane `crates/t/src/data_store/ingest.rs` must stay consistent.\n\n`crates/t/src/providers/store.rs` must not change and is never edited.\n",
        ),
    )
    .unwrap();
    let found = warnings(&tasks).unwrap().unwrap();
    assert!(found.is_empty(), "{found:#?}");
    assert!(section(Some(&tasks)).contains("none:"));
}

#[test]
fn a_task_set_without_a_record_is_not_checked() {
    let temp = tempfile::tempdir().unwrap();
    assert_eq!(warnings(temp.path()).unwrap(), None);
    assert!(section(Some(temp.path())).contains("not checked"));
}

/// The live task set, copied: `ARCHON_LINT_TASKS=<copied task dir>`.
#[test]
#[ignore = "needs a copied task set: ARCHON_LINT_TASKS"]
fn the_copied_live_task_set() {
    let Some(tasks) = std::env::var_os("ARCHON_LINT_TASKS") else {
        return;
    };
    println!("{}", section(Some(Path::new(&tasks))));
}
