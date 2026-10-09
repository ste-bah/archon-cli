use super::*;

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

#[tokio::test]
async fn live_freeze_reuses_unchanged_check_and_reruns_after_repository_input_changes() {
    let fixture = fixture_with(true, "test -f round2marker");
    std::fs::write(fixture.repo.path().join("round2marker"), "first\n").unwrap();
    std::fs::write(fixture.repo.path().join("missing"), "pass\n").unwrap();
    std::fs::write(fixture.repo.path().join("also-missing"), "pass\n").unwrap();
    git(
        fixture.repo.path(),
        &["add", "round2marker", "missing", "also-missing"],
    );
    git(fixture.repo.path(), &["commit", "-qm", "input"]);
    let command_digest =
        archon_workflow::task_set_contract::content_digest(b"test -f round2marker");
    let count = || super::super::exec::checks::test_execution_count(&command_digest);
    let before = count();

    for round in 1..=2 {
        run(&fixture, &execution(round, 4, &[]))
            .await
            .expect("the real live acceptance round completes");
    }
    assert_eq!(count() - before, 1, "unchanged verdict was reused");

    std::fs::write(fixture.repo.path().join("round2marker"), "changed\n").unwrap();
    git(fixture.repo.path(), &["add", "round2marker"]);
    git(fixture.repo.path(), &["commit", "-qm", "changed input"]);
    run(&fixture, &execution(3, 4, &[]))
        .await
        .expect("the changed repository input is checked");
    assert_eq!(count() - before, 2, "changed input forced execution");

    run(&fixture, &execution(4, 4, &[]))
        .await
        .expect("the unchanged changed-input round completes");
    assert_eq!(count() - before, 2, "the new verdict was reused");
}
