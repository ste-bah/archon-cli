use super::*;
use archon_workflow::repository_record::{RepositoryRecordV1, write_repository_record};
use archon_workflow::task_universe::WorkflowV2TaskUniverseTask;
use std::process::Command;

fn git(dir: &Path, args: &[&str]) {
    let output = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .output()
        .expect("git starts");
    assert!(
        output.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

/// A committed repository and a task directory recording it.
fn recorded() -> (tempfile::TempDir, PathBuf, PathBuf, RepositoryRecordV1) {
    let temp = tempfile::tempdir().unwrap();
    let repo = temp.path().join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    std::fs::write(repo.join("Cargo.toml"), "[package]\nname = \"x\"\n").unwrap();
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
        decomposition_run_id: "wf-decompose".into(),
        recorded_at: "now".into(),
    };
    write_repository_record(&tasks, &record).unwrap();
    (temp, repo, tasks, record)
}

fn universe(tasks: &Path) -> WorkflowV2TaskUniverse {
    WorkflowV2TaskUniverse {
        schema_version: "v1".into(),
        source_roots: vec![tasks.display().to_string()],
        tasks: vec![WorkflowV2TaskUniverseTask {
            canonical_task_id: "TASK-X-001".into(),
            source_path: tasks.join("TASK-X-001.md").display().to_string(),
            ..Default::default()
        }],
    }
}

#[test]
fn a_task_set_with_a_record_is_implemented_against_the_recorded_repository() {
    let (_temp, repo, tasks, record) = recorded();
    let resolution =
        resolve_target_repository("implement decomposed PRD", Some(&universe(&tasks))).unwrap();
    assert_eq!(
        resolution.target_repository_root.as_deref(),
        Some(record.repository_root.as_str())
    );
    let binding = resolution.binding.expect("bound");
    assert_eq!(binding.recorded_base_commit, record.base_commit);
    assert_eq!(binding.head, record.base_commit);
    assert!(!binding.drifted());
    assert_eq!(binding.record_path, tasks.join(REPOSITORY_LOCK_FILE));
    assert_eq!(binding.event_detail()["drift"], false);
    assert!(!binding.summary_line().contains("drift"));

    // The same repository named in the task text is not a contradiction.
    let named = format!(
        "implement decomposed PRD against the repository {}",
        repo.display()
    );
    assert!(resolve_target_repository(&named, Some(&universe(&tasks))).is_ok());
}

#[test]
fn a_task_naming_a_different_repository_is_refused() {
    let (temp, _repo, tasks, record) = recorded();
    let other = temp.path().join("other");
    std::fs::create_dir_all(&other).unwrap();
    let task = format!(
        "implement decomposed PRD against the repository {}",
        other.display()
    );
    let error = resolve_target_repository(&task, Some(&universe(&tasks)))
        .unwrap_err()
        .to_string();
    assert!(error.contains("names repository"), "{error}");
    assert!(error.contains(&record.repository_root), "{error}");
    assert!(error.contains(&other.display().to_string()), "{error}");
}

#[test]
fn a_moved_head_is_recorded_as_drift_not_refused() {
    let (_temp, repo, tasks, record) = recorded();
    std::fs::write(repo.join("more.txt"), "").unwrap();
    git(&repo, &["add", "."]);
    git(&repo, &["commit", "-q", "-m", "more"]);
    let binding = resolve_target_repository("implement decomposed PRD", Some(&universe(&tasks)))
        .unwrap()
        .binding
        .expect("bound");
    assert!(binding.drifted());
    assert_ne!(binding.head, record.base_commit);
    let detail = binding.event_detail();
    assert_eq!(detail["drift"], true);
    assert_eq!(detail["recorded_base_commit"], record.base_commit);
    assert_eq!(detail["head"], binding.head);
    let line = binding.summary_line();
    assert!(
        line.contains(&record.base_commit) && line.contains(&binding.head),
        "{line}"
    );
}

#[test]
fn a_task_set_without_a_record_keeps_the_inference() {
    let temp = tempfile::tempdir().unwrap();
    // The inference walks up from the task directory looking for markers;
    // nest deep enough that it cannot reach anything outside the fixture.
    let repo = temp.path().join("a/b/c/repo");
    let tasks = repo.join("tasks/PRD-X");
    std::fs::create_dir_all(&tasks).unwrap();
    std::fs::write(repo.join("Cargo.toml"), "[package]\n").unwrap();
    let resolution =
        resolve_target_repository("implement decomposed PRD", Some(&universe(&tasks))).unwrap();
    assert_eq!(
        resolution.target_repository_root,
        Some(repo.display().to_string())
    );
    assert_eq!(resolution.binding, None);
}

#[test]
fn a_recorded_repository_that_is_gone_is_an_error_and_two_records_must_agree() {
    let (temp, repo, tasks, _record) = recorded();
    let second = temp.path().join("project/tasks/PRD-Y");
    std::fs::create_dir_all(&second).unwrap();
    let mut other = archon_workflow::repository_record::read_repository_record(&tasks)
        .unwrap()
        .unwrap();
    other.repository_root = temp.path().join("elsewhere").display().to_string();
    write_repository_record(&second, &other).unwrap();
    let mut both = universe(&tasks);
    both.source_roots.push(second.display().to_string());
    let error = resolve_target_repository("implement", Some(&both))
        .unwrap_err()
        .to_string();
    assert!(
        error.contains("one run implements one repository"),
        "{error}"
    );

    std::fs::remove_dir_all(&repo).unwrap();
    let error = resolve_target_repository("implement", Some(&universe(&tasks)))
        .unwrap_err()
        .to_string();
    assert!(error.contains("no longer there"), "{error}");
}

#[test]
fn the_generated_plan_carries_the_binding_and_the_recorded_root() {
    let (_temp, _repo, tasks, record) = recorded();
    let plan = super::super::workflow_live_planner::WorkflowScriptPlan::generated(
        "implement decomposed PRD",
        "async function workflow(w) {}",
        Vec::new(),
        Some(universe(&tasks)),
        archon_core::config::GeneratedWorkflowConfig::default(),
        &archon_core::config::LearningConfig::default(),
    )
    .expect("plan");
    assert_eq!(
        plan.target_repository_root.as_deref(),
        Some(record.repository_root.as_str())
    );
    assert_eq!(
        plan.repository_binding
            .as_ref()
            .map(|b| b.recorded_base_commit.as_str()),
        Some(record.base_commit.as_str())
    );
    assert_eq!(
        plan.approval_metadata_spec()
            .target_repository_root
            .as_deref(),
        Some(record.repository_root.as_str())
    );
}
