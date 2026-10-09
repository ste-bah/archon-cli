use super::*;
use archon_workflow::task_set_contract::{
    AcceptanceCheck, AcceptanceCriterion, JudgeDecision, JudgeVerdict, TrustedCwd,
};

fn git(repo: &std::path::Path, args: &[&str]) {
    let output = std::process::Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(args)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn criterion() -> AcceptanceCriterion {
    AcceptanceCriterion {
        id: "AC-I-001".into(),
        criterion: "the required file exists".into(),
        check: AcceptanceCheck::Command {
            command: "test -f data/ignored.txt".into(),
            cwd: TrustedCwd::RepoRoot,
        },
        gap_permitted: false,
        covers: Vec::new(),
        judgment: JudgeVerdict {
            verdict: JudgeDecision::Accepted,
            counterexample: "the file is missing".into(),
            reason: "the check rejects a missing file".into(),
            sampling: None,
            host_call_id: "judge".into(),
        },
    }
}

fn criterion_for(command: &str) -> AcceptanceCriterion {
    AcceptanceCriterion {
        check: AcceptanceCheck::Command {
            command: command.into(),
            cwd: TrustedCwd::RepoRoot,
        },
        ..criterion()
    }
}

fn context(repo: &std::path::Path) -> StageContext {
    StageContext {
        project: repo.to_path_buf(),
        task_root: repo.to_path_buf(),
        repository: repo.to_path_buf(),
        binding: None,
        launch: None,
        launch_lineage: archon_workflow::task_set_lineage::LaunchLineage::Predates,
        run_id: "reuse-test".into(),
    }
}

fn clean_repo() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    git(dir.path(), &["init", "-q"]);
    git(
        dir.path(),
        &["config", "user.email", "test@example.invalid"],
    );
    git(dir.path(), &["config", "user.name", "test"]);
    std::fs::write(dir.path().join("tracked"), "fixture").unwrap();
    git(dir.path(), &["add", "tracked"]);
    git(dir.path(), &["commit", "-qm", "fixture"]);
    dir
}

#[test]
fn acceptance_freeze_reuse_logic_empty_untracked_directory_creation_and_deletion_invalidate_reuse()
{
    let dir = clean_repo();
    let context = context(dir.path());
    let check = criterion_for("test -d generated");
    let absent = super::reuse::key(&context, &check).expect("absent path is readable");
    std::fs::create_dir(dir.path().join("generated")).unwrap();
    let present = super::reuse::key(&context, &check).expect("empty directory is readable");
    assert_ne!(
        absent, present,
        "the key must commit to a real directory state"
    );
    std::fs::remove_dir(dir.path().join("generated")).unwrap();
    let deleted = super::reuse::key(&context, &check).expect("absent path is readable");
    assert_eq!(absent, deleted, "returning to the original state is stable");
}

#[test]
fn acceptance_freeze_reuse_logic_file_content_change_invalidates_reuse_and_unchanged_file_reuses() {
    let dir = clean_repo();
    let context = context(dir.path());
    std::fs::write(dir.path().join("input"), "one").unwrap();
    let check = criterion_for("test -f input");
    let first = super::reuse::key(&context, &check).expect("file state is readable");
    let same = super::reuse::key(&context, &check).expect("file state is readable");
    assert_eq!(first, same, "unchanged inputs must retain their key");
    let result = CheckResult {
        acceptance_id: check.id.clone(),
        exit_code: Some(0),
        quota_walk_count: 0,
        stdout: Vec::new(),
        stderr: Vec::new(),
        environment_note: None,
        operational_error: None,
        classification: None,
    };
    super::reuse::save(first.clone(), &result, "evidence".into());
    let (saved, decision, _) = super::reuse::take(&context, &check);
    assert!(saved.is_some() && decision.reused, "unchanged state reuses");
    std::fs::write(dir.path().join("input"), "two").unwrap();
    let (saved, decision, _) = super::reuse::take(&context, &check);
    assert!(saved.is_none() && !decision.reused, "changed bytes rerun");
}

#[test]
fn acceptance_freeze_reuse_logic_glob_predicate_is_volatile() {
    let dir = clean_repo();
    let check = criterion_for("test -f *.txt");
    assert!(super::reuse::key(&context(dir.path()), &check).is_none());
}

#[test]
fn acceptance_freeze_reuse_logic_verdict_and_key_come_from_one_snapshot() {
    let dir = clean_repo();
    let context = context(dir.path());
    let check = criterion_for("test -f appeared");
    let snapshot = super::reuse::snapshot(&context, &check).expect("literal snapshot");
    let key = snapshot.key().to_owned();
    let result = super::reuse::evaluate_snapshot(&snapshot, &check).unwrap();
    super::reuse::save(key.clone(), &result, "snapshot evidence".into());
    std::fs::write(dir.path().join("appeared"), "now present").unwrap();

    let (cached, decision, cached_key) = super::reuse::take_snapshot(&snapshot);
    assert!(decision.reused);
    assert_eq!(cached_key.as_deref(), Some(key.as_str()));
    assert_eq!(cached.unwrap().exit_code, Some(1));
    assert_eq!(snapshot.key(), key);
    assert_eq!(
        result.environment_note.as_deref(),
        Some("host-evaluated from filesystem snapshot")
    );
}

#[test]
fn acceptance_freeze_reuse_logic_literal_predicates_match_sh_test_for_same_snapshot() {
    let dir = clean_repo();
    let repo = dir.path();
    std::fs::write(repo.join("regular"), "data").unwrap();
    std::fs::create_dir(repo.join("directory")).unwrap();
    std::fs::write(repo.join("empty"), "").unwrap();
    #[cfg(unix)]
    std::os::unix::fs::symlink("regular", repo.join("link")).unwrap();

    let mut commands = vec![
        "test -e regular",
        "test -f regular",
        "test -d directory",
        "test -s regular",
        "test -r regular",
        "test -w regular",
        "test -x regular",
        "test -e missing",
        "test -f missing",
        "test -d missing",
        "test -s empty",
        "test -r missing",
        "test -w missing",
        "test -x missing",
    ];
    #[cfg(unix)]
    commands.extend(["test -L link", "test -e link", "test -f link"]);
    for command in commands {
        let check = criterion_for(command);
        let snapshot = super::reuse::snapshot(&context(repo), &check)
            .unwrap_or_else(|| panic!("unsupported bounded command: {command}"));
        let host = super::reuse::evaluate_snapshot(&snapshot, &check).unwrap();
        let shell = std::process::Command::new("sh")
            .args(["-c", command])
            .current_dir(repo)
            .status()
            .unwrap();
        assert_eq!(host.exit_code, shell.code(), "{command}");
    }
}

#[test]
fn acceptance_freeze_reuse_logic_non_normalized_spellings_are_volatile() {
    for spelling in [
        "regular/",
        "./regular",
        "nested/../regular",
        "nested//regular",
        "/regular",
    ] {
        let command = format!("test -f {spelling}");
        assert!(
            crate::command::workflow_task_set::workflow_acceptance_check_reuse::bounded_path(
                &command
            )
            .is_none(),
            "shell-significant spelling must be volatile: {spelling}"
        );
    }
}

#[test]
fn acceptance_freeze_reuse_logic_snapshot_differential_matrix_matches_sh_or_is_volatile() {
    let dir = clean_repo();
    let repo = dir.path();
    std::fs::write(repo.join("file"), "data").unwrap();
    std::fs::write(repo.join("empty"), "").unwrap();
    std::fs::create_dir(repo.join("directory")).unwrap();
    std::fs::create_dir(repo.join("nested")).unwrap();
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink("file", repo.join("link-file")).unwrap();
        std::os::unix::fs::symlink("directory", repo.join("link-directory")).unwrap();
        std::os::unix::fs::symlink("absent-target", repo.join("dangling")).unwrap();
    }

    let mut states = vec!["missing", "file", "empty", "directory"];
    #[cfg(unix)]
    states.extend(["link-file", "link-directory", "dangling"]);
    let operators = ["-e", "-f", "-d", "-s", "-L", "-r", "-w", "-x"];
    let spellings = [
        "{state}",
        "{state}/",
        "./{state}",
        "nested/../{state}",
        "nested//{state}",
        "/{state}",
    ];
    for state in states {
        for operator in operators {
            for spelling in spellings {
                let path = spelling.replace("{state}", state);
                let command = format!("test {operator} {path}");
                let shell = std::process::Command::new("sh")
                    .args(["-c", &command])
                    .current_dir(repo)
                    .status()
                    .unwrap();
                let check = criterion_for(&command);
                let host = super::reuse::snapshot(&context(repo), &check)
                    .and_then(|snapshot| super::reuse::evaluate_snapshot(&snapshot, &check))
                    .and_then(|result| result.exit_code);
                assert!(
                    host.is_none() || host == shell.code(),
                    "host verdict {host:?} disagrees with sh {:?} for {command}",
                    shell.code()
                );
            }
        }
    }
}

#[cfg(unix)]
#[test]
fn acceptance_freeze_reuse_logic_size_test_follows_symlink_target() {
    let dir = clean_repo();
    std::fs::write(dir.path().join("file"), "data").unwrap();
    std::os::unix::fs::symlink("file", dir.path().join("link")).unwrap();
    let check = criterion_for("test -s link");
    let snapshot = super::reuse::snapshot(&context(dir.path()), &check).unwrap();
    let result = super::reuse::evaluate_snapshot(&snapshot, &check).unwrap();
    assert_eq!(result.exit_code, Some(0));
}

#[test]
fn ignored_input_is_volatile_and_cannot_leave_a_verdict_after_deletion() {
    let dir = tempfile::tempdir().unwrap();
    git(dir.path(), &["init", "-q"]);
    git(
        dir.path(),
        &["config", "user.email", "test@example.invalid"],
    );
    git(dir.path(), &["config", "user.name", "test"]);
    std::fs::create_dir(dir.path().join("data")).unwrap();
    std::fs::write(dir.path().join(".gitignore"), "data/ignored.txt\n").unwrap();
    std::fs::write(dir.path().join("data/marker"), "tracked").unwrap();
    std::fs::write(dir.path().join("data/ignored.txt"), "present").unwrap();
    git(dir.path(), &["add", ".gitignore", "data/marker"]);
    git(dir.path(), &["commit", "-qm", "fixture"]);
    let context = StageContext {
        project: dir.path().to_path_buf(),
        task_root: dir.path().to_path_buf(),
        repository: dir.path().to_path_buf(),
        binding: None,
        launch: None,
        launch_lineage: archon_workflow::task_set_lineage::LaunchLineage::Predates,
        run_id: "reuse-test".into(),
    };
    assert!(
        super::reuse::key(&context, &criterion()).is_none(),
        "an ignored file is visible to the check but absent from HEAD"
    );

    std::fs::remove_file(dir.path().join("data/ignored.txt")).unwrap();
    let (saved, decision, key) = super::reuse::take(&context, &criterion());
    assert!(
        saved.is_none(),
        "the earlier ignored-file verdict was not saved"
    );
    assert!(!decision.reused);
    assert!(key.is_some(), "the now-absent path has a closed read set");
}
