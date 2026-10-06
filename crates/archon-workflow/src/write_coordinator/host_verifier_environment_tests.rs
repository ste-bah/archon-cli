#![cfg(unix)]

use super::*;

fn operator(case: &str, path: bool, check: impl FnOnce(&Path)) {
    if std::env::var("ISSUE_349_CASE").as_deref() == Ok(case) {
        check(Path::new(&std::env::var("HOME").unwrap()));
        return;
    }
    let root = tempfile::tempdir().unwrap();
    let mut child = archon_shell::spawn::command(std::env::current_exe().unwrap());
    child
        .args([case, "--nocapture"])
        .env_clear()
        .env("ISSUE_349_CASE", case)
        .env("HOME", root.path())
        .env("JAVA_HOME", root.path())
        .env("OPERATOR_SECRET", "hidden")
        .env("FIXTURE_API_KEY", "hidden-data")
        .env("HTTPS_PROXY", "https://user:password@proxy.invalid");
    if path {
        child.env("PATH", "/usr/bin:/bin");
    }
    let output = child.output().unwrap();
    assert!(
        output.status.success(),
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn artifact_and_contract_builder_withholds_operator_secrets() {
    operator(
        "artifact_and_contract_builder_withholds_operator_secrets",
        true,
        |root| {
            let (mut child, boundary, _) = command(Path::new("/bin/sh"), None, &[]).unwrap();
            let output = child.args(["-c", "test -z \"${OPERATOR_SECRET-}${FIXTURE_API_KEY-}${HTTPS_PROXY-}\" && test \"$HOME\" = \"$JAVA_HOME\""]).current_dir(root).output().unwrap();
            boundary.finish("test").unwrap();
            assert!(output.status.success());
        },
    );
}

#[test]
fn artifact_and_contract_builder_missing_path_cannot_run() {
    operator(
        "artifact_and_contract_builder_missing_path_cannot_run",
        false,
        |_| match command(Path::new("/bin/sh"), None, &[]) {
            Err(reason) => assert!(
                reason.contains("environment") && reason.contains("PATH"),
                "{reason}"
            ),
            Ok(_) => panic!("built verifier without PATH"),
        },
    );
}

#[test]
fn focused_baseline_withholds_secrets_and_preserves_host_overlay() {
    operator(
        "focused_baseline_withholds_secrets_and_preserves_host_overlay",
        true,
        |root| {
            let dispatch =
                crate::v2::write::test_baseline_run_base::tests::FakeCargo { bin: root.into() };
            let runtime = tokio::runtime::Runtime::new().unwrap();
            let result = runtime.block_on(crate::v2::write::test_baseline_run::run_in_worktree(
            &dispatch, root,
            "test -z \"${OPERATOR_SECRET-}${FIXTURE_API_KEY-}${HTTPS_PROXY-}\" && test \"$HOME\" = \"$JAVA_HOME\" && test \"${PATH%%:*}\" = \"$HOME\"",
            None,
        ));
            assert!(result.error.is_none(), "{:?}", result.error);
            assert_eq!(result.exit_code, Some(0), "{}", result.output);
            let split = runtime.block_on(crate::v2::write::test_baseline_run::run_in_worktree(
                &dispatch, root,
                "test -z \"${FIXTURE_API_KEY-}\" || exit 4; printf FIXTURE_API_KEY; printf ' is required' >&2; exit 3",
                None,
            ));
            assert!(split.error.is_none(), "{:?}", split.error);
            assert_eq!(split.exit_code, Some(3), "{}", split.output);
        },
    );
}
