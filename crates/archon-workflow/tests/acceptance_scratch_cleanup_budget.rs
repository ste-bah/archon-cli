#[path = "support/native_fixture.rs"]
mod support;

#[test]
#[ignore = "subprocess with private Git wrapper"]
fn cleanup_budget_child() {
    let (_temp, mut policy, commit, _, _) = support::fixture("test -f input");
    policy.timeout_secs = 20;
    let mut roots = archon_workflow::acceptance_scratch::ScratchRoots::prepare(&policy, &commit).unwrap();
    let owned = roots.root().to_path_buf();
    roots.cleanup().expect("cleanup must use the host profile budget, not five seconds");
    assert!(!owned.exists());
}

#[test]
fn cleanup_uses_profile_budget_for_slow_owned_tree_removal() {
    use std::os::unix::fs::PermissionsExt;
    let tools = tempfile::tempdir().unwrap();
    let real_git = std::process::Command::new("/usr/bin/which").arg("git").output().unwrap();
    assert!(real_git.status.success());
    let real_git = String::from_utf8(real_git.stdout).unwrap();
    let wrapper = tools.path().join("git");
    std::fs::write(&wrapper, format!(
        "#!/bin/sh\ncase \" $* \" in *' worktree remove '*) /bin/sleep 6;; esac\nexec '{}' \"$@\"\n", real_git.trim()
    )).unwrap();
    std::fs::set_permissions(&wrapper, std::fs::Permissions::from_mode(0o755)).unwrap();
    let mut child = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "cleanup_budget_child", "--ignored", "--nocapture"])
        .env("PATH", format!("{}:{}",tools.path().display(),std::env::var("PATH").unwrap()))
        .spawn().unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(40);
    loop {
        if let Some(status)=child.try_wait().unwrap() { assert!(status.success()); break; }
        if std::time::Instant::now()>=deadline { child.kill().unwrap();child.wait().unwrap();panic!("cleanup test deadline"); }
        std::thread::sleep(std::time::Duration::from_millis(25));
    }
}
