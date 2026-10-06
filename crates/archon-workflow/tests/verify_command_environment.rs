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

fn recorded_policy(root: &std::path::Path, allowlist: &[&str]) {
    std::fs::create_dir_all(root.join("v2")).unwrap();
    std::fs::write(
        root.join("v2/generated-metadata.json"),
        serde_json::to_vec(&serde_json::json!({
            "observer_snapshot": {"native_execution": {"policy": {
                "repository": root, "project": root, "task_root": root.join("tasks"),
                "scratch_parent": root.join("scratch"), "project_inputs": [], "combined": true,
                "toolchain_path": "/usr/bin:/bin", "environment": {"LANG": "C"},
                "environment_allowlist": allowlist, "cargo_seed": null,
                "timeout_secs": 10, "output_bytes": 1024, "scratch_bytes": 1024
            }}}
        }))
        .unwrap(),
    )
    .unwrap();
}

#[test]
fn review349_wave_consumes_recorded_data_policy() {
    operator(
        "review349_wave_consumes_recorded_data_policy",
        true,
        |root| {
            recorded_policy(root, &["FIXTURE_API_KEY"]);
            run_wave_verify(root, Some("test \"$FIXTURE_API_KEY\" = data-secret && test -z \"${OPERATOR_SECRET-}${HTTPS_PROXY-}\" && test \"$LANG\" = C"), 1, root, "stage").unwrap();
        },
    );
}

#[test]
fn review349_wave_missing_allowlisted_data_refuses_before_launch() {
    operator(
        "review349_wave_missing_allowlisted_data_refuses_before_launch",
        true,
        |root| {
            recorded_policy(root, &["ABSENT_API_KEY"]);
            let error = run_wave_verify(root, Some("touch launched"), 1, root, "stage")
                .unwrap_err()
                .to_string();
            assert!(error.contains("ABSENT_API_KEY"), "{error}");
            assert!(!root.join("launched").exists());
        },
    );
}

#[test]
fn review349_wave_malformed_policy_never_falls_back() {
    operator(
        "review349_wave_malformed_policy_never_falls_back",
        true,
        |root| {
            std::fs::create_dir_all(root.join("v2")).unwrap();
            for broken in [
                "{bad",
                "[]",
                r#"{"observer_snapshot":"bad"}"#,
                r#"{"observer_snapshot":{"native_execution":{"capture_error":"invalid operator policy"}}}"#,
                r#"{"observer_snapshot":{"native_execution":{}}}"#,
            ] {
                std::fs::write(root.join("v2/generated-metadata.json"), broken).unwrap();
                let error = run_wave_verify(root, Some("touch launched"), 1, root, "stage")
                    .unwrap_err()
                    .to_string();
                assert!(
                    error.contains("policy") && error.contains("generated-metadata.json"),
                    "{error}"
                );
                assert!(!root.join("launched").exists());
            }
        },
    );
}

#[test]
fn review349_stage_explicit_policy_reaches_verifier() {
    operator(
        "review349_stage_explicit_policy_reaches_verifier",
        true,
        |root| {
            let host = archon_workflow::acceptance_check_environment::host_environment();
            let mut policy =
                archon_workflow::acceptance_check_environment::CheckPolicy::default_for(&host);
            policy.forwarded = vec!["FIXTURE_API_KEY".into()];
            archon_workflow::acceptance::run_verify_command_with_policy(
                root,
                Some("test \"$FIXTURE_API_KEY\" = data-secret && test -z \"${OPERATOR_SECRET-}\""),
                Some(&policy),
            )
            .unwrap();
            for name in ["ABSENT_API_KEY", "BASH_ENV", "NODE_OPTIONS"] {
                policy.forwarded = vec![name.into()];
                let error = archon_workflow::acceptance::run_verify_command_with_policy(
                    root,
                    Some("touch launched"),
                    Some(&policy),
                )
                .unwrap_err();
                assert!(error.contains(name), "{error}");
                assert!(!root.join("launched").exists());
            }
        },
    );
}
