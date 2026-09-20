use super::*;
use std::process::Command;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

const PRD: &str = "# PRD X\n\n## Requirements\n\n| ID | Requirement |\n|---|---|\n| REQ-X-001 | Prove the fixture. |\n\n## Acceptance Criteria\n\n| ID | Criterion |\n|---|---|\n| AC-X-001 | The fixture is proven. |\n";

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

/// `<temp>/project` (PRD and task root, not a repository) beside
/// `<temp>/repo` (a checkout with one commit).
fn layout() -> (tempfile::TempDir, PathBuf, PathBuf) {
    let temp = tempfile::tempdir().unwrap();
    let project = temp.path().join("project");
    std::fs::create_dir_all(project.join("prds")).unwrap();
    std::fs::create_dir_all(project.join("tasks/PRD-X")).unwrap();
    std::fs::write(project.join("prds/PRD-X.md"), PRD).unwrap();
    let repo = temp.path().join("repo");
    std::fs::create_dir_all(repo.join("src")).unwrap();
    std::fs::write(repo.join("src/lib.rs"), "pub fn f() {}\n").unwrap();
    git(&repo, &["init", "-q"]);
    git(&repo, &["config", "user.email", "t@example.invalid"]);
    git(&repo, &["config", "user.name", "t"]);
    git(&repo, &["add", "."]);
    git(&repo, &["commit", "-q", "-m", "initial"]);
    (temp, project, repo)
}

#[test]
fn the_flag_wins_then_workflow_config_then_acceptance_execution_then_refusal() {
    let (temp, project, repo) = layout();
    let other = temp.path().join("other");
    std::fs::create_dir_all(&other).unwrap();
    git(&other, &["init", "-q"]);
    let mut config = ArchonConfig::default();

    let error = resolve_repository(&project, None, &config).unwrap_err().to_string();
    assert!(error.contains("--repository <PATH>"), "{error}");
    assert!(error.contains("[workflow] repository_root"), "{error}");
    assert!(error.contains("never assumed"), "{error}");

    config.workflow.acceptance_execution = Some(archon_core::config::AcceptanceExecutionConfig {
        repository: other.clone(),
        scratch_parent: temp.path().join("scratch"),
        project_inputs: Vec::new(),
        project_input_excludes: Vec::new(),
        project_repository_view: Default::default(),
        toolchain_path: String::new(),
        environment: Default::default(),
        environment_allowlist: Vec::new(),
        cargo_seed: None,
        timeout_secs: 1,
        output_bytes: 1,
        scratch_bytes: 1,
    });
    let resolved = resolve_repository(&project, None, &config).unwrap();
    assert_eq!(resolved.source, RepositorySource::AcceptanceExecutionConfig);
    assert_eq!(resolved.root, other.canonicalize().unwrap());
    assert_eq!(resolved.base_commit, "unborn", "a fresh repository is valid");

    config.workflow.repository_root = Some(repo.clone());
    let resolved = resolve_repository(&project, None, &config).unwrap();
    assert_eq!(resolved.source, RepositorySource::WorkflowConfig);
    assert_eq!(resolved.root, repo.canonicalize().unwrap());
    assert_eq!(resolved.base_commit.len(), 40);

    let resolved = resolve_repository(&project, Some(&other), &config).unwrap();
    assert_eq!(resolved.source, RepositorySource::Flag);
    assert_eq!(resolved.root, other.canonicalize().unwrap());
}

#[test]
fn a_relative_path_resolves_against_cwd_and_a_non_checkout_or_missing_directory_is_refused() {
    let (temp, project, repo) = layout();
    let config = ArchonConfig::default();
    let resolved = resolve_repository(&project, Some(Path::new("../repo")), &config).unwrap();
    assert_eq!(resolved.root, repo.canonicalize().unwrap());

    let plain = temp.path().join("plain");
    std::fs::create_dir_all(&plain).unwrap();
    let error = resolve_repository(&project, Some(&plain), &config).unwrap_err().to_string();
    assert!(error.contains("not a git checkout"), "{error}");
    assert!(error.contains("--repository"), "{error}");

    let error = resolve_repository(&project, Some(Path::new("nowhere")), &config)
        .unwrap_err()
        .to_string();
    assert!(error.contains("not an existing directory"), "{error}");

    let mut config = ArchonConfig::default();
    config.workflow.repository_root = Some(plain);
    let error = resolve_repository(&project, None, &config).unwrap_err().to_string();
    assert!(error.contains("[workflow] repository_root"), "{error}");
}

#[test]
fn an_existing_record_must_name_the_same_repository_and_reports_a_moved_base() {
    let (temp, project, repo) = layout();
    let tasks = project.join("tasks/PRD-X");
    let config = ArchonConfig::default();
    let resolved = resolve_repository(&project, Some(&repo), &config).unwrap();
    assert_eq!(verify_existing_record(&tasks, &resolved).unwrap(), None);

    let record = record_launch(&tasks, &resolved, "wf-first").unwrap();
    assert_eq!(record.decomposition_run_id, "wf-first");
    assert_eq!(record.base_commit, resolved.base_commit);
    let again = verify_existing_record(&tasks, &resolved).unwrap().expect("record");
    assert_eq!(again, record);
    assert_eq!(drift_text(&record, &resolved), None);
    assert!(!log_line("wf-first", &resolved, &record).contains("drift=true"));

    // The base moves: reported, not refused, and the record is untouched.
    std::fs::write(repo.join("src/more.rs"), "").unwrap();
    git(&repo, &["add", "."]);
    git(&repo, &["commit", "-q", "-m", "more"]);
    let moved = resolve_repository(&project, Some(&repo), &config).unwrap();
    assert_ne!(moved.base_commit, record.base_commit);
    let verified = verify_existing_record(&tasks, &moved).unwrap().expect("record");
    assert_eq!(verified, record);
    let drift = drift_text(&record, &moved).expect("drift");
    assert!(drift.contains(&record.base_commit) && drift.contains(&moved.base_commit), "{drift}");
    let line = log_line("wf-second", &moved, &record);
    assert!(line.starts_with("event=repository_grounded run_id=wf-second "), "{line}");
    assert!(line.contains("drift=true"), "{line}");
    assert!(line.contains(&format!("recorded_base_commit={}", record.base_commit)), "{line}");

    // A different repository path refuses.
    let other = temp.path().join("other");
    std::fs::create_dir_all(&other).unwrap();
    git(&other, &["init", "-q"]);
    let elsewhere = resolve_repository(&project, Some(&other), &config).unwrap();
    let error = verify_existing_record(&tasks, &elsewhere).unwrap_err().to_string();
    assert!(error.contains("records repository"), "{error}");
    assert!(error.contains(&record.repository_root), "{error}");
    assert!(error.contains("--repository"), "{error}");
}

/// A factory that must never be reached: the launch is expected to refuse first.
struct NeverFactory {
    builds: AtomicUsize,
}

#[async_trait::async_trait(?Send)]
impl archon_workflow::WorkflowLlmClientFactory for NeverFactory {
    async fn build_client(
        &self,
        _request: archon_workflow::WorkflowLlmClientRequest,
    ) -> archon_workflow::WorkflowResult<Arc<dyn archon_workflow::WorkflowLlmClient>> {
        self.builds.fetch_add(1, Ordering::SeqCst);
        Err(archon_workflow::WorkflowError::port("reached the provider"))
    }
}

#[tokio::test]
async fn a_launch_without_a_repository_refuses_before_any_run_exists() {
    let (_temp, project, _repo) = layout();
    let factory = NeverFactory {
        builds: AtomicUsize::new(0),
    };
    let error = crate::command::workflow_decompose::run_fixed_decomposition_with_factory(
        &project,
        Path::new("prds/PRD-X.md"),
        Path::new("tasks/PRD-X"),
        None,
        true,
        &ArchonConfig::default(),
        &archon_core::env_vars::load_env_vars_from(&std::collections::HashMap::new()),
        &factory,
    )
    .await
    .unwrap_err();
    assert_eq!(error.to_string(), NO_REPOSITORY_REMEDY);
    assert_eq!(factory.builds.load(Ordering::SeqCst), 0);
    assert!(!project.join(".archon/workflows").exists(), "no run was created");
    assert!(!project.join("tasks/PRD-X").join(REPOSITORY_LOCK_FILE).exists());
}

#[tokio::test]
async fn a_launch_grounded_by_flag_records_the_repository_and_a_relaunch_elsewhere_refuses() {
    let (temp, project, repo) = layout();
    let tasks = project.join("tasks/PRD-X");
    let factory = NeverFactory {
        builds: AtomicUsize::new(0),
    };
    let env = archon_core::env_vars::load_env_vars_from(&std::collections::HashMap::new());
    let error = crate::command::workflow_decompose::run_fixed_decomposition_with_factory(
        &project,
        Path::new("prds/PRD-X.md"),
        Path::new("tasks/PRD-X"),
        Some(&repo),
        true,
        &ArchonConfig::default(),
        &env,
        &factory,
    )
    .await
    .unwrap_err();
    assert!(format!("{error:#}").contains("reached the provider"), "{error:#}");
    assert_eq!(factory.builds.load(Ordering::SeqCst), 1);
    let record = read_repository_record(&tasks).unwrap().expect("record written");
    assert_eq!(record.repository_root, path_text(&repo.canonicalize().unwrap()));
    assert_eq!(record.base_commit, git_head(&repo).unwrap());
    let log = std::fs::read_to_string(tasks.join(".decompose.log")).unwrap();
    let grounded = log
        .lines()
        .find(|line| line.starts_with("event=repository_grounded"))
        .expect("the operator log names the repository");
    assert!(grounded.contains(&format!("run_id={}", record.decomposition_run_id)), "{grounded}");
    assert!(grounded.contains("source=--repository"), "{grounded}");

    // The same task root, a different repository: refused before a run exists.
    let other = temp.path().join("other");
    std::fs::create_dir_all(&other).unwrap();
    git(&other, &["init", "-q"]);
    let store = archon_workflow::WorkflowStore::project(project.canonicalize().unwrap());
    let runs_before = store.list_runs().unwrap().len();
    let error = crate::command::workflow_decompose::run_fixed_decomposition_with_factory(
        &project,
        Path::new("prds/PRD-X.md"),
        Path::new("tasks/PRD-X"),
        Some(&other),
        true,
        &ArchonConfig::default(),
        &env,
        &factory,
    )
    .await
    .unwrap_err();
    assert!(error.to_string().contains("records repository"), "{error:#}");
    assert_eq!(factory.builds.load(Ordering::SeqCst), 1, "the provider is not reached");
    assert_eq!(store.list_runs().unwrap().len(), runs_before, "no second run");
    assert_eq!(read_repository_record(&tasks).unwrap().as_ref(), Some(&record), "the record is untouched");
}
