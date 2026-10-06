//! Run each case in a separate operator process; never mutate the test host's environment.
#![cfg(unix)]

use archon_workflow::acceptance::run_verify_command;
use archon_workflow::write_coordinator::patch_apply::run_wave_verify;

fn operator(case: &str, path: bool, check: impl FnOnce(&std::path::Path)) {
    if std::env::var("ISSUE_349_CASE").as_deref() == Ok(case) {
        check(std::path::Path::new(&std::env::var("HOME").unwrap()));
        return;
    }
    let home = tempfile::tempdir().unwrap();
    let mut child = archon_shell::spawn::command(std::env::current_exe().unwrap());
    child
        .args(["--exact", case, "--nocapture"])
        .env_clear()
        .env("ISSUE_349_CASE", case)
        .env("HOME", home.path())
        .env("LANG", "C")
        .env("JAVA_HOME", home.path())
        .env("OPERATOR_SECRET", "do-not-forward")
        .env("UNCLASSIFIED_VALUE", "also-private")
        .env("FIXTURE_API_KEY", "data-secret")
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

const SAFE: &str = "test -z \"${OPERATOR_SECRET-}${UNCLASSIFIED_VALUE-}${FIXTURE_API_KEY-}${HTTPS_PROXY-}\" && test \"$HOME\" = \"$JAVA_HOME\" && test \"$LANG\" = C && test -n \"$PATH\"";

#[test]
fn stage_withholds_secrets_and_keeps_locators() {
    operator("stage_withholds_secrets_and_keeps_locators", true, |root| {
        run_verify_command(root, Some(SAFE)).unwrap();
    });
}

#[test]
fn wave_withholds_secrets_and_keeps_locators() {
    operator("wave_withholds_secrets_and_keeps_locators", true, |root| {
        run_wave_verify(root, Some(SAFE), 1, root, "stage").unwrap();
    });
}

#[test]
fn stage_missing_path_refuses_before_execution() {
    operator(
        "stage_missing_path_refuses_before_execution",
        false,
        |root| {
            let error = run_verify_command(root, Some("touch launched")).unwrap_err();
            assert!(
                error.contains("environment") && error.contains("PATH"),
                "{error}"
            );
            assert!(!root.join("launched").exists());
        },
    );
}

#[test]
fn wave_missing_path_refuses_before_execution() {
    operator(
        "wave_missing_path_refuses_before_execution",
        false,
        |root| {
            let error = run_wave_verify(root, Some("touch launched"), 1, root, "stage")
                .unwrap_err()
                .to_string();
            assert!(
                error.contains("environment") && error.contains("PATH"),
                "{error}"
            );
            assert!(!root.join("launched").exists());
        },
    );
}

#[test]
fn stage_reports_forwardable_missing_data_without_values() {
    operator(
        "stage_reports_forwardable_missing_data_without_values",
        true,
        |root| {
            let error = run_verify_command(root, Some("test -n \"${FIXTURE_API_KEY-}\" || { echo 'FIXTURE_API_KEY is not set' >&2; exit 1; }")).unwrap_err();
            assert!(
                error.contains("FIXTURE_API_KEY") && error.contains("no verdict"),
                "{error}"
            );
            assert!(!error.contains("data-secret"));
        },
    );
}

#[test]
fn wave_reports_forwardable_missing_data_without_values() {
    operator(
        "wave_reports_forwardable_missing_data_without_values",
        true,
        |root| {
            let error = run_wave_verify(root, Some("test -n \"${FIXTURE_API_KEY-}\" || { echo 'FIXTURE_API_KEY is not set' >&2; exit 1; }"), 1, root, "stage").unwrap_err().to_string();
            assert!(
                error.contains("FIXTURE_API_KEY") && error.contains("no verdict"),
                "{error}"
            );
            assert!(!error.contains("data-secret"));
        },
    );
}

#[test]
fn stage_bare_mentions_and_forbidden_names_stay_product_failures() {
    operator(
        "stage_bare_mentions_and_forbidden_names_stay_product_failures",
        true,
        |root| {
            for diagnostic in [
                "FIXTURE_API_KEY fixture exists",
                "UNCLASSIFIED_VALUE is not set",
                "expected 'FIXTURE_API_KEY is required' in stderr",
            ] {
                let script = format!(
                    "test -z \"${{FIXTURE_API_KEY-}}\" || exit 4; echo \"{diagnostic}\" >&2; exit 3"
                );
                let error = run_verify_command(root, Some(&script)).unwrap_err();
                assert!(
                    error.contains("status 3") && !error.contains("no verdict"),
                    "{error}"
                );
            }
        },
    );
}
