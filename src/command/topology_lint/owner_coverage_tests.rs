use super::*;
use archon_workflow::repository_record::{RepositoryRecordV1, git_head, write_repository_record};
use archon_workflow::task_skeleton::FrozenTask;
use archon_workflow::task_universe::WorkflowV2DeliverableContract;
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

/// A repository holding `crates/x/src/lib.rs`, `crates/x/src/util.rs`,
/// `docs/guide.md` and `Cargo.toml`, recorded beside an empty task root.
fn grounded() -> (tempfile::TempDir, PathBuf, RepositoryTree) {
    let temp = tempfile::tempdir().unwrap();
    let repo = temp.path().join("repo");
    std::fs::create_dir_all(repo.join("crates/x/src")).unwrap();
    std::fs::create_dir_all(repo.join("docs")).unwrap();
    std::fs::write(repo.join("crates/x/src/lib.rs"), "pub mod util;\n").unwrap();
    std::fs::write(repo.join("crates/x/src/util.rs"), "pub fn f() {}\n").unwrap();
    std::fs::write(repo.join("docs/guide.md"), "# guide\n").unwrap();
    std::fs::write(repo.join("Cargo.toml"), "[workspace]\n").unwrap();
    git(&repo, &["init", "-q"]);
    git(&repo, &["config", "user.email", "t@example.invalid"]);
    git(&repo, &["config", "user.name", "t"]);
    git(&repo, &["add", "."]);
    git(&repo, &["commit", "-q", "-m", "initial"]);
    let tasks = temp.path().join("project/tasks/PRD-X");
    std::fs::create_dir_all(&tasks).unwrap();
    let record = RepositoryRecordV1 {
        schema_version: 1,
        repository_root: repo.canonicalize().unwrap().display().to_string(),
        base_commit: git_head(&repo).unwrap(),
        decomposition_run_id: "wf-test".into(),
        recorded_at: "now".into(),
    };
    write_repository_record(&tasks, &record).unwrap();
    let tree = RepositoryTree::load(&record).unwrap();
    (temp, tasks, tree)
}

fn skeleton(owned: &[&str]) -> TaskSkeleton {
    TaskSkeleton {
        schema_version: 1,
        acceptance_digest: String::new(),
        tasks: vec![FrozenTask {
            task_id: "TASK-X-001".into(),
            file_name: "TASK-X-001.md".into(),
            depends_on: Vec::new(),
            blocks: Vec::new(),
            implements: vec!["AC-X-001".into()],
            deliverable_contracts: owned
                .iter()
                .map(|path| WorkflowV2DeliverableContract {
                    kind: "source".into(),
                    artifact_path: (*path).into(),
                    ..Default::default()
                })
                .collect(),
        }],
    }
}

const PRD: &str = "# PRD\n\nExtend `crates/x/src/lib.rs` and the helpers in crates/x/src/util.rs; see docs/guide.md and [the manifest](Cargo.toml).\nNot paths: e.g. v1.2, `src/*.rs`, https://example.com/x.md, `crates/x/src/missing.rs`.\n\n```\ncrates/x/src/lib.rs is inside a fence and ignored\n```\n";

#[test]
fn only_path_literals_that_exist_at_the_base_are_named() {
    let (_temp, _tasks, tree) = grounded();
    let named: Vec<String> = prd_named_repository_paths(&tree, PRD).into_iter().collect();
    assert_eq!(
        named,
        vec![
            "Cargo.toml".to_string(),
            "crates/x/src/lib.rs".to_string(),
            "crates/x/src/util.rs".to_string(),
            "docs/guide.md".to_string(),
        ]
    );
}

#[test]
fn an_unowned_prd_named_file_is_a_blocking_skeleton_finding() {
    let (_temp, tasks, tree) = grounded();
    let findings = skeleton_findings(
        &tasks,
        PRD,
        &skeleton(&["crates/x/src/lib.rs", "docs/guide.md", "Cargo.toml"]),
    )
    .unwrap();
    assert_eq!(findings.len(), 1, "{findings:?}");
    assert!(findings[0].starts_with("repository file `crates/x/src/util.rs` is named by the PRD"), "{}", findings[0]);
    assert!(findings[0].contains(&format!("base commit {}", tree.base_commit())), "{}", findings[0]);
}

#[test]
fn ownership_covers_directories_both_ways_and_absolute_owned_paths() {
    let (_temp, tasks, tree) = grounded();
    // The PRD names a directory: a task owning any path under it owns it.
    let prd = "Refactor `crates/x/src/`.";
    assert!(skeleton_findings(&tasks, prd, &skeleton(&["crates/x/src/util.rs"])).unwrap().is_empty());
    // The task owns a directory above the PRD-named file.
    assert!(skeleton_findings(&tasks, PRD, &skeleton(&["crates/x", "docs", "Cargo.toml"])).unwrap().is_empty());
    // An owned path cited absolutely under the repository root counts.
    let absolute = format!("{}/crates/x/src/util.rs", tree.root().display());
    let findings = skeleton_findings(
        &tasks,
        PRD,
        &skeleton(&[absolute.as_str(), "crates/x/src/lib.rs", "docs/guide.md", "Cargo.toml"]),
    )
    .unwrap();
    assert!(findings.is_empty(), "{findings:?}");
    // Nothing owned: every named path is a finding.
    assert_eq!(skeleton_findings(&tasks, PRD, &skeleton(&[])).unwrap().len(), 4);
}

#[test]
fn a_task_set_without_a_record_is_not_checked() {
    let temp = tempfile::tempdir().unwrap();
    assert!(skeleton_findings(temp.path(), PRD, &skeleton(&[])).unwrap().is_empty());
}

#[test]
fn the_set_gate_counts_files_expected_to_change_and_names_the_skeleton() {
    let (temp, tasks, _tree) = grounded();
    let project = temp.path().join("project");
    std::fs::write(project.join("tasks/PRD-X.md"), PRD).unwrap();
    std::fs::write(
        tasks.join("TASK-X-001.md"),
        "# TASK-X-001\n\n```yaml\ntask_id: TASK-X-001\ntitle: T\ncomplexity: low\nstatus: ready\ndepends_on: []\nblocks: []\nimplements: [\"AC-X-001\"]\nrequired_env_keys: []\nrequired_tools: []\ndeliverable_contracts: []\n```\n\n## Files Expected to Change\n\n- crates/x/src/lib.rs\n- crates/x/src/util.rs\n- Cargo.toml\n\n## Focused Tests\n\n- `cargo test -p x`\n",
    )
    .unwrap();
    let findings = set_findings(&tasks).unwrap();
    assert_eq!(findings.len(), 1, "{:?}", findings.iter().map(|f| &f.text).collect::<Vec<_>>());
    assert_eq!(findings[0].subject, "docs/guide.md");
    assert_eq!(findings[0].remediation_scope, archon_workflow::RemediationScope::Skeleton);
}
