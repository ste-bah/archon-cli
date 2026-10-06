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
        .env("LANG", "C")
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

fn run_script(root: &Path, script: &str) -> Ran {
    run(
        root,
        &["/bin/sh".into(), "-c".into(), script.into()],
        Duration::from_secs(5),
    )
}

#[test]
fn falsifier_withholds_secrets_and_keeps_locators() {
    operator(
        "falsifier_withholds_secrets_and_keeps_locators",
        true,
        |root| {
            assert!(matches!(
                run_script(
                    root,
                    "test -z \"${OPERATOR_SECRET-}${FIXTURE_API_KEY-}${HTTPS_PROXY-}\" && test \"$HOME\" = \"$JAVA_HOME\" && test \"$LANG\" = C"
                ),
                Ran::Finished { success: true, .. }
            ));
            // Joining these streams would invent a missing-key claim neither contains.
            assert!(matches!(
                run_script(
                    root,
                    "test -z \"${FIXTURE_API_KEY-}\" || exit 4; printf FIXTURE_API_KEY; printf ' is required' >&2; exit 3"
                ),
                Ran::Finished {
                    code: Some(3),
                    success: false,
                    ..
                }
            ));
        },
    );
}

#[test]
fn falsifier_missing_path_refuses_before_execution() {
    operator(
        "falsifier_missing_path_refuses_before_execution",
        false,
        |root| {
            match run_script(root, "touch launched") {
                Ran::NotLaunchable { reason } => assert!(
                    reason.contains("environment") && reason.contains("PATH"),
                    "{reason}"
                ),
                _ => panic!("ran without PATH"),
            }
            assert!(!root.join("launched").exists());
        },
    );
}

#[test]
fn falsifier_missing_data_is_inconclusive() {
    operator(
        "falsifier_missing_data_is_inconclusive",
        true,
        |root| match run_script(
            root,
            "test -n \"${FIXTURE_API_KEY-}\" || { echo 'FIXTURE_API_KEY is not set' >&2; exit 1; }",
        ) {
            Ran::NotLaunchable { reason } => {
                assert!(
                    reason.contains("FIXTURE_API_KEY") && reason.contains("no verdict"),
                    "{reason}"
                );
                assert!(!reason.contains("hidden-data"));
            }
            _ => panic!("missing operator data became a product verdict"),
        },
    );
}
