use super::*;
use std::cell::Cell;
thread_local! { static FAIL: Cell<bool> = const { Cell::new(false) }; }
pub(crate) fn fire(path: &Path) -> WorkflowResult<()> {
    if FAIL.replace(false) {
        return Err(WorkflowError::io(
            path,
            std::io::Error::from_raw_os_error(5),
        ));
    }
    Ok(())
}

/// Absolute toolchain directories in this host's own PATH syntax. On Windows
/// `/usr/bin` names no drive, so it is not absolute and the policy refuses it;
/// the host's own absolute PATH entries hold the POSIX tools the checks run.
fn toolchain_path() -> String {
    if cfg!(windows) {
        let host = std::env::var_os("PATH").unwrap_or_default();
        let absolute = std::env::split_paths(&host).filter(|path| path.is_absolute());
        std::env::join_paths(absolute)
            .expect("host PATH entries rejoin")
            .into_string()
            .expect("host PATH is Unicode")
    } else {
        "/usr/bin:/bin".into()
    }
}

// A check runs only in a fully prepared scratch site. Off Unix, preparation
// always refuses before any check is spawned ("native scratch target links
// require a supported Unix host", `acceptance_scratch.rs`), so no check can
// exit with a status there; the integration targets that run checks are
// Unix-only for the same reason. The setup-fault cleanup test below runs on
// every host.
#[cfg(unix)]
#[tokio::test]
async fn acceptance_scratch_rebuild_preserves_native_io_and_completed_findings() {
    tokio::time::timeout(std::time::Duration::from_secs(20), async {
        for authored in [false, true] {
            let temp = tempfile::tempdir().unwrap();
            let repository = temp.path().join("repository");
            let project = temp.path().join("workspace");
            let task_root = project.join("items");
            std::fs::create_dir_all(&repository).unwrap();
            std::fs::create_dir_all(&task_root).unwrap();
            std::fs::write(repository.join("input"), "data").unwrap();
            for args in [
                vec!["init", "-q"], vec!["add", "."],
                vec!["-c", "user.email=test@example.invalid", "-c", "user.name=test", "commit", "-qm", "base"],
            ] {
                assert!(std::process::Command::new("git").arg("-C").arg(&repository).args(args).status().unwrap().success());
            }
            let commit = String::from_utf8(std::process::Command::new("git").arg("-C").arg(&repository).args(["rev-parse", "HEAD"]).output().unwrap().stdout).unwrap().trim().to_owned();
            let policy = ScratchPolicy {
                repository, project, task_root, scratch_parent: temp.path().join("scratch"),
                project_inputs: vec![], project_input_excludes: vec![], combined: false,
                toolchain_path: toolchain_path(), environment: BTreeMap::new(),
                environment_allowlist: vec![], cargo_seed: None, timeout_secs: 30,
                output_bytes: 4096, scratch_bytes: 16777216, build_cache: None,
            };
            let first = if authored { "rm input; false" } else { "false" };
            let contract: AcceptanceContract = serde_json::from_value(serde_json::json!({
                "schema_version":1,"prd":{"path":"input.md","digest":"d"},"gap_policy":{},
                "acceptance": ([("AC-X-001", first), ("AC-X-002", "test -f input")].map(|(id, command)| serde_json::json!({
                    "id": id,"criterion":"observable output", "check":{"kind":"command","command":command,"cwd":"repo_root"},"gap_permitted":false,
                    "judgment":{"verdict":"accepted","counterexample":"missing","reason":"declared","host_call_id":"call"}
                })))
            })).unwrap();
            let digest = content_digest(&serde_json::to_vec(&contract).unwrap());
            let refs = [("AC-X-001", first), ("AC-X-002", "test -f input")].map(|(id, command)| FrozenCommandRef {
                acceptance_id: id.into(), kind: crate::acceptance_world::AcceptanceCommandKind::Command,
                chain_digest: digest.clone(), command_digest: content_digest(command.as_bytes()),
            });
            let scratch = policy.scratch_parent.clone();
            let mut hooks = ObserveHooks::for_resource_limits(Default::default());
            hooks.on_check = Some(std::sync::Arc::new(move |check| {
                if check.acceptance_id == "AC-X-001" {
                    if !authored {
                        let root = std::fs::read_dir(&scratch).unwrap().next().unwrap().unwrap().path();
                        std::fs::write(root.join(SITE_REPOSITORY_DIRECTORY).join("input"), "changed").unwrap();
                    }
                    FAIL.set(true);
                }
            }));
            let observed = observe_commands_hooked(&policy, &commit, &contract, &digest, &refs,
                &temp.path().join("evidence"), ObserveControl::new(Default::default(), hooks)).await.unwrap();
            assert!(!FAIL.get(), "preparation must fail during restoration between checks");
            assert!(observed.teardown_verified && observed.live_roots_unchanged, "{observed:?}");
            assert_eq!(observed.checks[0].exit_code, Some(1));
            assert_eq!(observed.checks[0].fault.is_some(), authored, "{observed:?}");
            let later = serde_json::to_value(&observed.checks[1]).unwrap();
            assert_eq!(later["fault"], serde_json::json!({"cause":"site_setup_io","raw_os_error":5}),
                "rebuild must preserve native setup identity: {later}");
            assert_eq!(std::fs::read_dir(&policy.scratch_parent).unwrap().count(), 0);
            let recovered = observe_commands(&policy, &commit, &contract, &digest, &refs[1..],
                &temp.path().join("recovered"), Default::default()).await.unwrap();
            assert!(recovered.passed(), "{recovered:?}");
        }
    }).await.expect("rebuild and recovery must finish");
}

#[tokio::test]
async fn host_command_scratch_setup_retains_native_io_after_verified_cleanup() {
    let temp = tempfile::tempdir().unwrap();
    let repository = temp.path().join("repository");
    let project = temp.path().join("workspace");
    let task_root = project.join("items");
    std::fs::create_dir_all(&repository).unwrap();
    std::fs::create_dir_all(&task_root).unwrap();
    std::fs::write(repository.join("input"), "data").unwrap();
    for args in [
        vec!["init", "-q"],
        vec!["add", "."],
        vec![
            "-c",
            "user.email=test@example.invalid",
            "-c",
            "user.name=test",
            "commit",
            "-qm",
            "base",
        ],
    ] {
        let status = std::process::Command::new("git")
            .arg("-C")
            .arg(&repository)
            .args(args)
            .status()
            .unwrap();
        assert!(status.success());
    }
    let commit = String::from_utf8(
        std::process::Command::new("git")
            .arg("-C")
            .arg(&repository)
            .args(["rev-parse", "HEAD"])
            .output()
            .unwrap()
            .stdout,
    )
    .unwrap()
    .trim()
    .to_owned();
    let policy = ScratchPolicy {
        repository,
        project,
        task_root,
        scratch_parent: temp.path().join("scratch"),
        project_inputs: vec![],
        project_input_excludes: vec![],
        combined: false,
        toolchain_path: toolchain_path(),
        environment: BTreeMap::new(),
        environment_allowlist: vec![],
        cargo_seed: None,
        timeout_secs: 30,
        output_bytes: 4096,
        scratch_bytes: 16777216,
        build_cache: None,
    };
    let contract: AcceptanceContract = serde_json::from_value(serde_json::json!({
        "schema_version":1,"prd":{"path":"input.md","digest":"d"},"gap_policy":{},
        "acceptance":[{"id":"AC-X-001","criterion":"observable output",
        "check":{"kind":"command","command":"test -f input","cwd":"repo_root"},"gap_permitted":false,
        "judgment":{"verdict":"accepted","counterexample":"missing","reason":"declared","host_call_id":"call"}}]
    })).unwrap();
    let digest = content_digest(&serde_json::to_vec(&contract).unwrap());
    let reference = FrozenCommandRef {
        acceptance_id: "AC-X-001".into(),
        kind: crate::acceptance_world::AcceptanceCommandKind::Command,
        chain_digest: digest.clone(),
        command_digest: content_digest(b"test -f input"),
    };
    FAIL.set(true);
    let result = tokio::time::timeout(
        std::time::Duration::from_secs(15),
        observe_commands(
            &policy,
            &commit,
            &contract,
            &digest,
            &[reference],
            &temp.path().join("evidence"),
            Default::default(),
        ),
    )
    .await
    .expect("setup must finish");
    let error = result.expect_err("safely cleaned setup must propagate its native error");
    assert!(
        matches!(error, WorkflowError::Io { ref source, .. } if source.raw_os_error() == Some(5)),
        "{error:?}"
    );
    assert_eq!(
        std::fs::read_dir(&policy.scratch_parent).unwrap().count(),
        0,
        "failed setup leaves no scratch tree"
    );
}
