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
