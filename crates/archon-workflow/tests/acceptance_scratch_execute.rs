use archon_workflow::acceptance_scratch::{ScratchPolicy, observe_commands};
use archon_workflow::acceptance_world::{AcceptanceCommandKind, FrozenCommandRef};
use archon_workflow::task_set_contract::{AcceptanceContract, content_digest};
use std::{collections::BTreeMap, path::Path, process::Command};
fn git(root: &Path, args: &[&str]) -> String {
    let o = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .output()
        .unwrap();
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    String::from_utf8(o.stdout).unwrap().trim().into()
}
fn fixture(
    command: &str,
) -> (
    tempfile::TempDir,
    ScratchPolicy,
    String,
    AcceptanceContract,
    Vec<FrozenCommandRef>,
) {
    let t = tempfile::tempdir().unwrap();
    let repo = t.path().join("repo");
    let project = t.path().join("project");
    std::fs::create_dir_all(&repo).unwrap();
    std::fs::create_dir_all(project.join("data")).unwrap();
    std::fs::create_dir_all(project.join("tasks")).unwrap();
    git(&repo, &["init", "-q"]);
    git(&repo, &["config", "user.email", "test@example.invalid"]);
    git(&repo, &["config", "user.name", "test"]);
    std::fs::write(repo.join("input"), "source").unwrap();
    git(&repo, &["add", "."]);
    git(&repo, &["commit", "-qm", "fixture"]);
    let commit = git(&repo, &["rev-parse", "HEAD"]);
    std::fs::write(project.join("data/value"), "before").unwrap();
    let p = ScratchPolicy {
        repository: repo,
        project: project.clone(),
        task_root: project.join("tasks"),
        scratch_parent: t.path().join("scratch"),
        project_inputs: vec!["data".into()],
        combined: true,
        toolchain_path: "/usr/bin:/bin:/usr/sbin:/sbin".into(),
        environment: BTreeMap::new(),
        cargo_seed: None,
        timeout_secs: 3,
        output_bytes: 2048,
        scratch_bytes: 16 * 1024 * 1024,
    };
    let c:AcceptanceContract=serde_json::from_value(serde_json::json!({"schema_version":1,"prd":{"path":"p","digest":"d"},"gap_policy":{"permitted_acceptance_ids":[],"forbidden_phrases":[],"required_fields":[]},"acceptance":[{"id":"AC-X-001","criterion":"output correct","check":{"kind":"command","command":command,"cwd":"project_root"},"judgment":{"verdict":"accepted","counterexample":"incorrect output","reason":"checks output","host_call_id":"j"}}]})).unwrap();
    let refs = vec![FrozenCommandRef {
        acceptance_id: "AC-X-001".into(),
        kind: AcceptanceCommandKind::Command,
        chain_digest: "chain".into(),
        command_digest: content_digest(command.as_bytes()),
    }];
    (t, p, commit, c, refs)
}
#[tokio::test]
async fn native_command_mutates_only_scratch_and_records_verified_cleanup() {
    let (t, p, commit, c, r) = fixture(
        "test -f data/value && printf after > data/value; test \"$(cat data/value)\" = after",
    );
    let out = observe_commands(&p, &commit, &c, "chain", &r, &t.path().join("evidence"))
        .await
        .unwrap();
    assert!(out.passed());
    assert_eq!(out.checks[0].exit_code, Some(0));
    assert!(out.live_roots_unchanged && out.teardown_verified);
    assert_eq!(
        std::fs::read_to_string(p.project.join("data/value")).unwrap(),
        "before"
    );
    assert_eq!(std::fs::read_dir(&p.scratch_parent).unwrap().count(), 0);
}
#[tokio::test]
async fn direct_live_write_voids_even_a_zero_exit() {
    let (t, p, commit, mut c, mut r) = fixture("test -f data/value");
    let cmd = format!(
        "test -f data/value && printf changed > '{}'; test -f data/value",
        p.project.join("data/value").display()
    );
    if let archon_workflow::task_set_contract::AcceptanceCheck::Command { command, .. } =
        &mut c.acceptance[0].check
    {
        *command = cmd.clone();
    }
    r[0].command_digest = content_digest(cmd.as_bytes());
    let out = observe_commands(&p, &commit, &c, "chain", &r, &t.path().join("evidence"))
        .await
        .unwrap();
    assert!(!out.passed());
    assert!(!out.live_roots_unchanged);
}
#[tokio::test]
async fn timeout_and_output_flood_are_operational_and_cleaned() {
    for cmd in ["sleep 30; test -f input", "while :; do printf flood; done"] {
        let (t, mut p, commit, c, r) = fixture(cmd);
        p.timeout_secs = 1;
        let out = observe_commands(&p, &commit, &c, "chain", &r, &t.path().join("evidence"))
            .await
            .unwrap();
        assert!(!out.passed());
        assert!(out.checks[0].operational_error.is_some());
        assert!(out.teardown_verified);
    }
}
#[tokio::test]
async fn authorization_failure_creates_no_scratch_and_runs_nothing() {
    let (t, p, commit, c, mut r) =
        fixture("test -f data/value && printf after > data/value; test -f input");
    r[0].command_digest = "wrong".into();
    assert!(
        observe_commands(&p, &commit, &c, "chain", &r, &t.path().join("evidence"))
            .await
            .is_err()
    );
    assert!(!p.scratch_parent.exists());
}

#[tokio::test]
async fn real_native_build_relative_binary_and_warm_target_work() {
    let cmd = "cargo build --offline --release && ./target/release/probe && test -s data/result";
    let (t, mut p, _, mut c, mut refs) = fixture(cmd);
    std::fs::create_dir_all(p.repository.join("src")).unwrap();
    std::fs::write(
        p.repository.join("Cargo.toml"),
        "[package]\nname=\"probe\"\nversion=\"0.1.0\"\nedition=\"2024\"\n",
    )
    .unwrap();
    std::fs::write(
        p.repository.join("src/main.rs"),
        "fn main(){std::fs::write(\"data/result\",\"built\").unwrap();}\n",
    )
    .unwrap();
    git(&p.repository, &["add", "."]);
    git(&p.repository, &["commit", "-qm", "probe"]);
    let commit = git(&p.repository, &["rev-parse", "HEAD"]);
    let rustc = Command::new("rustup")
        .args(["which", "rustc"])
        .output()
        .unwrap();
    assert!(rustc.status.success());
    let bin = std::path::PathBuf::from(String::from_utf8(rustc.stdout).unwrap().trim())
        .parent()
        .unwrap()
        .to_path_buf();
    p.toolchain_path = format!("{}:/usr/bin:/bin:/usr/sbin:/sbin", bin.display());
    p.timeout_secs = 60;
    p.scratch_bytes = 256 * 1024 * 1024;
    let mut second = c.acceptance[0].clone();
    second.id = "AC-X-002".into();
    c.acceptance.push(second);
    let mut second = refs[0].clone();
    second.acceptance_id = "AC-X-002".into();
    refs.push(second);
    let result = observe_commands(&p, &commit, &c, "chain", &refs, &t.path().join("evidence"))
        .await
        .unwrap();
    assert!(result.passed(), "{result:?}");
    assert!(String::from_utf8_lossy(&result.checks[0].stderr).contains("Compiling probe"));
    assert!(!String::from_utf8_lossy(&result.checks[1].stderr).contains("Compiling probe"));
    assert!(!p.project.join("data/result").exists());
}

#[tokio::test]
async fn ambient_credentials_and_shell_startup_are_not_inherited() {
    let cmd = "test -z \"${NATIVE_SECRET_CANARY:-}\" && test -z \"${BASH_ENV:-}\" && test \"$HOME\" != / && test -d \"$CARGO_HOME\"";
    let (t, p, commit, c, r) = fixture(cmd);
    // Use the test executable as a subprocess for environment isolation.
    let out = observe_commands(&p, &commit, &c, "chain", &r, &t.path().join("evidence"))
        .await
        .unwrap();
    assert!(out.passed(), "{out:?}");
}

#[tokio::test]
async fn native_check_reaches_an_owned_loopback_service() {
    use std::io::{Read, Write};
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    listener.set_nonblocking(true).unwrap();
    let server = std::thread::spawn(move || {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        loop {
            match listener.accept() {
                Ok((mut stream, _)) => {
                    stream
                        .set_read_timeout(Some(std::time::Duration::from_secs(2)))
                        .unwrap();
                    let mut buf = [0; 4096];
                    let _ = stream.read(&mut buf);
                    stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\nConnection: close\r\n\r\nready").unwrap();
                    break;
                }
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    assert!(
                        std::time::Instant::now() < deadline,
                        "service not contacted"
                    );
                    std::thread::sleep(std::time::Duration::from_millis(10));
                }
                Err(e) => panic!("{e}"),
            }
        }
    });
    let cmd = format!("curl --fail --silent --max-time 3 http://127.0.0.1:{port}/ | grep -q ready");
    let (t, p, commit, c, r) = fixture(&cmd);
    let result = observe_commands(&p, &commit, &c, "chain", &r, &t.path().join("evidence"))
        .await
        .unwrap();
    assert!(result.passed(), "{result:?}");
    server.join().unwrap();
}

#[tokio::test]
async fn normal_nonzero_is_not_an_operational_failure() {
    let (t, p, commit, c, r) = fixture("test -f absent");
    let result = observe_commands(&p, &commit, &c, "chain", &r, &t.path().join("evidence"))
        .await
        .unwrap();
    assert!(!result.passed());
    assert_eq!(result.checks[0].exit_code, Some(1));
    assert!(result.checks[0].operational_error.is_none());
    assert!(result.teardown_verified);
}

#[tokio::test]
async fn short_command_cannot_escape_scratch_size_check_by_exiting() {
    let (t, mut p, commit, c, r) =
        fixture("test -f input && dd if=/dev/zero of=large bs=1048576 count=2 2>/dev/null");
    p.scratch_bytes = 1024 * 1024;
    let result = observe_commands(&p, &commit, &c, "chain", &r, &t.path().join("evidence"))
        .await
        .unwrap();
    assert!(!result.passed());
    assert!(
        result.checks[0]
            .operational_error
            .as_ref()
            .unwrap()
            .contains("scratch")
    );
}

#[tokio::test]
async fn changed_scratch_source_cannot_feed_a_later_check() {
    let (t, p, commit, mut c, mut r) = fixture("test -f input && printf modified > input");
    let mut second = c.acceptance[0].clone();
    second.id = "AC-X-002".into();
    second.check=serde_json::from_value(serde_json::json!({"kind":"command","command":"grep -q modified input","cwd":"project_root"})).unwrap();
    c.acceptance.push(second);
    r.push(FrozenCommandRef {
        acceptance_id: "AC-X-002".into(),
        kind: AcceptanceCommandKind::Command,
        chain_digest: "chain".into(),
        command_digest: content_digest(b"grep -q modified input"),
    });
    let result = observe_commands(&p, &commit, &c, "chain", &r, &t.path().join("evidence"))
        .await
        .unwrap();
    assert!(
        !result.passed(),
        "changed source must invalidate observation rather than reuse its target"
    );
    assert_eq!(
        result.checks.len(),
        1,
        "later command must not execute against modified source"
    );
    assert!(
        result.checks[0]
            .operational_error
            .as_ref()
            .unwrap()
            .contains("source")
    );
}

#[tokio::test]
async fn evidence_binds_commands_policy_and_copied_inputs() {
    let (t, p, commit, c, refs) = fixture("test -f data/value");
    let evidence = t.path().join("evidence");
    observe_commands(&p, &commit, &c, "chain", &refs, &evidence)
        .await
        .unwrap();
    let record: serde_json::Value =
        serde_json::from_slice(&std::fs::read(evidence.join("observation.json")).unwrap()).unwrap();
    assert_eq!(
        record["command_refs"][0]["command_digest"],
        refs[0].command_digest
    );
    assert_eq!(record["command_cwds"][0], "project_root");
    assert_eq!(record["policy"]["timeout_secs"], p.timeout_secs);
    assert_eq!(record["policy"]["toolchain_path"], p.toolchain_path);
    assert!(
        record["copied_project_manifest"]["data/value"]
            .as_str()
            .unwrap()
            .contains(&content_digest(b"before"))
    );
}

#[tokio::test]
async fn separate_view_uses_each_declared_cwd_without_live_inputs() {
    use archon_workflow::task_set_contract::{AcceptanceCheck, TrustedCwd};
    let cmd = "test -f data/value && test ! -e input";
    let (t, mut p, commit, mut c, mut refs) = fixture(cmd);
    p.combined = false;
    let mut second = c.acceptance[0].clone();
    second.id = "AC-X-002".into();
    let repo_command = "test -f input && test -L target && test ! -e data/value";
    second.check = AcceptanceCheck::Command {
        command: repo_command.into(),
        cwd: TrustedCwd::RepoRoot,
    };
    c.acceptance.push(second);
    let mut reference = refs[0].clone();
    reference.acceptance_id = "AC-X-002".into();
    reference.command_digest = content_digest(repo_command.as_bytes());
    refs.push(reference);
    let out = observe_commands(&p, &commit, &c, "chain", &refs, &t.path().join("evidence"))
        .await
        .unwrap();
    assert!(out.passed(), "{out:?}");
    assert_eq!(out.checks.len(), 2);
}
