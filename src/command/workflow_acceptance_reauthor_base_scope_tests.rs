use super::*;
use crate::command::workflow_task_set::executability::materialise_base_copy;
use archon_workflow::base_snapshot::snapshot_deliverables;
use archon_workflow::repository_record::{RepositoryRecordV1, RepositoryTree};
fn git(root: &Path, args: &[&str]) -> String {
    let out = archon_shell::spawn::command("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8(out.stdout).unwrap().trim().into()
}
#[test]
fn base_scope_confines_roots_and_rejects_live_project() {
    for nested in [false, true] {
        let repository = tempfile::tempdir().unwrap();
        let parent = tempfile::tempdir().unwrap();
        git(repository.path(), &["init", "-q"]);
        git(repository.path(), &["config", "user.name", "test"]);
        git(
            repository.path(),
            &["config", "user.email", "test@example.invalid"],
        );
        let project = if nested {
            repository.path().join("sub")
        } else {
            repository.path().into()
        };
        std::fs::create_dir_all(&project).unwrap();
        std::fs::write(project.join("value"), b"base").unwrap();
        git(repository.path(), &["add", "."]);
        git(repository.path(), &["commit", "-qm", "fixture"]);
        let commit = git(repository.path(), &["rev-parse", "HEAD"]);
        let tree = RepositoryTree::load(&RepositoryRecordV1 {
            schema_version: 1,
            repository_root: repository.path().display().to_string(),
            base_commit: commit.clone(),
            decomposition_run_id: "test".into(),
            recorded_at: "test".into(),
        })
        .unwrap();
        let key = if nested { "sub/value" } else { "value" };
        let snapshot = snapshot_deliverables(&tree, repository.path(), [key]).unwrap();
        let copy = materialise_base_copy(
            parent.path(),
            repository.path(),
            &project,
            None,
            &snapshot,
            &commit,
            &parent.path().join("output"),
            None,
        )
        .unwrap();
        let prd = parent.path().join("input");
        std::fs::write(&prd, b"requirement").unwrap();
        let universe = archon_workflow::task_universe::WorkflowV2TaskUniverse {
            schema_version: "workflow-v2-task-universe-v1".into(),
            source_roots: vec![],
            tasks: vec![],
        };
        let mut scope = for_base_copy(&copy, &prd, &parent.path().join("tasks"), universe);
        assert_eq!(scope.repository_root, copy.repository());
        assert_eq!(scope.project_root, copy.project());
        assert_eq!(scope.baseline.as_ref().unwrap().commit, commit);
        assert!(scope.project_input_excludes.is_empty());
        assert_eq!(
            confined_read_roots(&scope, copy.root()).unwrap(),
            vec![prd.display().to_string()]
        );
        scope.project_root = project.clone();
        assert_eq!(
            confined_read_roots(&scope, copy.root()),
            Err(format!(
                "acceptance author scope: project_root {} lies outside the base copy {}",
                project.display(),
                copy.root().display()
            ))
        );
    }
}
