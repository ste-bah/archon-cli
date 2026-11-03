use super::*;
use crate::coding::gates::E2ESmokeTestGate;
use std::os::unix::fs::PermissionsExt;

async fn probe(smoke: bool, safe: bool, name: &str) {
    if std::env::var("ARCHON_TEST_SECRET").as_deref() != Ok("sk-test-355") {
        let status = archon_shell::spawn::command(std::env::current_exe().unwrap())
            .args([
                "--exact",
                &format!("coding::compilation_gate::agent_environment_tests::{name}"),
                "--nocapture",
            ])
            .env("ARCHON_TEST_SECRET", "sk-test-355")
            .env("LANG", "C")
            .env("TMPDIR", std::env::temp_dir())
            .status()
            .unwrap();
        assert!(status.success(), "gate environment regression: {name}");
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let script = dir.path().join("probe");
    std::fs::write(&script, "#!/bin/sh\nif [ -z \"${ARCHON_TEST_SECRET+x}\" ]; then echo withheld; fi\nif [ -n \"$PATH\" ] && [ -n \"$HOME\" ] && [ \"$LANG\" = C ] && [ -n \"$TMPDIR\" ]; then echo safe; fi\n").unwrap();
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o700)).unwrap();
    let output = if smoke {
        E2ESmokeTestGate
            .run(dir.path(), script.to_str().unwrap(), None)
            .await
            .evidence
    } else {
        match execute(
            CommandSpec::new(&script, [], dir.path()),
            Duration::from_secs(30),
        )
        .await
        .unwrap()
        {
            CommandExecution::Completed(output) => String::from_utf8(output.stdout).unwrap(),
            _ => panic!("gate did not complete"),
        }
    };
    assert!(
        output.contains(if safe { "safe" } else { "withheld" }),
        "required environment property absent"
    );
}
macro_rules! regression {
    ($name:ident, $smoke:literal, $safe:literal) => {
        #[tokio::test]
        async fn $name() {
            probe($smoke, $safe, stringify!($name)).await;
        }
    };
}
regression!(agent_environment_compilation_secret, false, false);
regression!(agent_environment_compilation_safe, false, true);
regression!(agent_environment_smoke_secret, true, false);
regression!(agent_environment_smoke_safe, true, true);

async fn suite_probe(safe: bool, name: &str) {
    if std::env::var("ARCHON_TEST_SECRET").as_deref() != Ok("sk-test-355") {
        let dir = tempfile::tempdir().unwrap();
        let npm = dir.path().join("npm");
        std::fs::write(&npm, "#!/bin/sh\nif [ -z \"${ARCHON_TEST_SECRET+x}\" ]; then echo withheld; fi\nif [ -n \"$PATH\" ] && [ -n \"$HOME\" ] && [ \"$LANG\" = C ] && [ -n \"$TMPDIR\" ]; then echo safe; fi\n").unwrap();
        std::fs::set_permissions(&npm, std::fs::Permissions::from_mode(0o700)).unwrap();
        let mut paths = vec![dir.path().to_path_buf()];
        paths.extend(std::env::split_paths(&std::env::var_os("PATH").unwrap()));
        let status = archon_shell::spawn::command(std::env::current_exe().unwrap())
            .args([
                "--exact",
                &format!("coding::compilation_gate::agent_environment_tests::{name}"),
                "--nocapture",
            ])
            .env("PATH", std::env::join_paths(paths).unwrap())
            .env("ARCHON_TEST_SECRET", "sk-test-355")
            .env("LANG", "C")
            .env("TMPDIR", std::env::temp_dir())
            .status()
            .unwrap();
        assert!(status.success(), "suite gate environment regression");
        return;
    }
    let result = crate::coding::gates::TestsRunGate
        .run(
            std::path::Path::new("."),
            crate::coding::gates::Language::TypeScript,
        )
        .await;
    assert!(result.gate_passed);
    assert!(
        result
            .evidence
            .contains(if safe { "safe" } else { "withheld" }),
        "required environment property absent"
    );
}
#[tokio::test]
async fn agent_environment_suite_secret() {
    suite_probe(false, "agent_environment_suite_secret").await;
}
#[tokio::test]
async fn agent_environment_suite_safe() {
    suite_probe(true, "agent_environment_suite_safe").await;
}

async fn r5_pipeline_write_probe(smoke: bool, isolated: bool) {
    use archon_tools::workflow_read_guard::{
        DeclaredTargetScope, HostWriteBoundary, ReadOnlyBoundaryScope, RunStoreScope,
        scope_declared_targets, scope_read_only_boundary, scope_run_store,
    };
    let dir = tempfile::tempdir().unwrap();
    let sealed = dir.path().join("checkout");
    let work = dir.path().join("work");
    std::fs::create_dir_all(&sealed).unwrap();
    let store = dir.path().join("run-store");
    let run = store.join("run");
    std::fs::create_dir_all(&run).unwrap();
    std::fs::create_dir(&work).unwrap();
    let checkout = sealed.join("source");
    let record = run.join("state.json");
    for path in [&checkout, &record] {
        std::fs::write(path, "sealed").unwrap();
    }
    let script = work.join("build");
    std::fs::write(
        &script,
        format!(
            "#!/bin/sh\nprintf changed > '{}'\nprintf changed > '{}'\nprintf built > '{}/built'\n",
            checkout.display(),
            record.display(),
            work.display()
        ),
    )
    .unwrap();
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o700)).unwrap();
    let scope = ReadOnlyBoundaryScope::new(
        work.to_str().unwrap(),
        &[sealed.display().to_string()],
        &[work.display().to_string()],
    )
    .unwrap();
    let operation = scope_run_store(
        RunStoreScope::new(run.to_str(), store.to_str(), work.to_str()),
        async {
            if smoke {
                let _ = E2ESmokeTestGate
                    .run(&work, script.to_str().unwrap(), None)
                    .await;
            } else {
                let _ = execute(
                    CommandSpec::new(&script, [], &work),
                    Duration::from_secs(30),
                )
                .await;
            }
            for path in [&checkout, &record] {
                assert_eq!(
                    std::fs::read_to_string(path).unwrap(),
                    "sealed",
                    "agent-written gate script escaped write boundary"
                );
            }
            if matches!(
                archon_shell::write_boundary::mechanism(),
                Ok(archon_shell::write_boundary::Mechanism::SandboxExec
                    | archon_shell::write_boundary::Mechanism::Landlock { .. })
            ) {
                assert_eq!(
                    std::fs::read_to_string(work.join("built")).unwrap(),
                    "built"
                );
            }
        },
    );
    if isolated {
        let targets = DeclaredTargetScope::new(&["build".into()], work.to_str())
            .in_isolated_worktree(true)
            .with_write_boundary(HostWriteBoundary::new(&[sealed.display().to_string()], &[]));
        scope_declared_targets(targets, operation).await;
    } else {
        scope_read_only_boundary(Some(scope), operation).await;
    }
}

#[tokio::test]
async fn r5_pipeline_build_script_cannot_write_sealed_checkout_or_store() {
    r5_pipeline_write_probe(false, false).await;
}

#[tokio::test]
async fn r5_pipeline_smoke_script_cannot_write_sealed_checkout_or_store() {
    r5_pipeline_write_probe(true, false).await;
}

#[tokio::test]
async fn r5_pipeline_isolated_write_branch_build_cannot_write_sealed_checkout_or_store() {
    r5_pipeline_write_probe(false, true).await;
}

#[tokio::test]
async fn r5_pipeline_suite_script_cannot_write_sealed_checkout_or_store() {
    use archon_tools::workflow_read_guard::{ReadOnlyBoundaryScope, scope_read_only_boundary};
    if let Some(root) = std::env::var_os("ARCHON_R5_SUITE_ROOT") {
        let root = PathBuf::from(root);
        let work = root.join("work");
        let scope = ReadOnlyBoundaryScope::new(
            work.to_str().unwrap(),
            &[root.join("checkout").display().to_string()],
            &[work.display().to_string()],
        )
        .unwrap();
        scope_read_only_boundary(Some(scope), async {
            let _ = crate::coding::gates::TestsRunGate
                .run(&work, crate::coding::gates::Language::TypeScript)
                .await;
        })
        .await;
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let sealed = dir.path().join("checkout");
    let work = dir.path().join("work");
    std::fs::create_dir_all(sealed.join("runs")).unwrap();
    std::fs::create_dir(&work).unwrap();
    for path in [sealed.join("source"), sealed.join("runs/state.json")] {
        std::fs::write(path, "sealed").unwrap();
    }
    let script = work.join("npm");
    std::fs::write(&script, format!("#!/bin/sh\nprintf changed > '{}/source'\nprintf changed > '{}/runs/state.json'\nprintf built > '{}/built'\n", sealed.display(), sealed.display(), work.display())).unwrap();
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o700)).unwrap();
    let mut paths = vec![work.clone()];
    paths.extend(std::env::split_paths(&std::env::var_os("PATH").unwrap()));
    let status = archon_shell::spawn::command(std::env::current_exe().unwrap())
        .args(["--exact", "coding::compilation_gate::agent_environment_tests::r5_pipeline_suite_script_cannot_write_sealed_checkout_or_store", "--nocapture"])
        .env("ARCHON_R5_SUITE_ROOT", dir.path()).env("PATH", std::env::join_paths(paths).unwrap()).status().unwrap();
    assert!(status.success());
    for path in [sealed.join("source"), sealed.join("runs/state.json")] {
        assert_eq!(
            std::fs::read_to_string(path).unwrap(),
            "sealed",
            "test script escaped write boundary"
        );
    }
}
