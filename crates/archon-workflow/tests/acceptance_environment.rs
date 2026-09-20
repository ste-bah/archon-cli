#[path = "support/native_fixture.rs"]
mod support;
use archon_workflow::acceptance_scratch::{ScratchPolicy, observe_commands};
#[test]
fn configured_host_values_cross_only_at_execution_and_never_enter_evidence() {
    let mut child = std::process::Command::new(std::env::current_exe().unwrap());
    child
        .args(["--exact", "environment_child", "--ignored", "--nocapture"])
        .env("FIXTURE_ALLOWED_TOKEN", "secret-acceptance-canary-98e3")
        .env("FIXTURE_NOT_ALLOWED", "ambient-must-not-cross");
    assert!(child.status().unwrap().success());
}
#[tokio::test]
#[ignore = "private environment subprocess"]
async fn environment_child() {
    let (t, p, commit, c, refs) = support::fixture(
        "test -n \"$FIXTURE_ALLOWED_TOKEN\" && test -z \"${FIXTURE_NOT_ALLOWED:-}\" && printf '%s' \"$FIXTURE_ALLOWED_TOKEN\"",
    );
    let mut raw = serde_json::to_value(&p).unwrap();
    raw["environment_allowlist"] = serde_json::json!(["FIXTURE_ALLOWED_TOKEN", "FIXTURE_ABSENT"]);
    let policy: ScratchPolicy = serde_json::from_value(raw).expect("allowlist must be supported");
    let evidence = t.path().join("evidence");
    let out = observe_commands(&policy, &commit, &c, "chain", &refs, &evidence)
        .await
        .unwrap();
    assert!(out.passed(), "{:?}", out.operational_errors);
    let encoded = serde_json::to_string(&out).unwrap();
    assert!(!encoded.contains("secret-acceptance-canary-98e3"));
    assert!(!format!("{out:?}").contains("secret-acceptance-canary-98e3"));
    assert!(
        !String::from_utf8_lossy(&out.checks[0].stdout).contains("secret-acceptance-canary-98e3")
    );
    let value: serde_json::Value = serde_json::from_str(&encoded).unwrap();
    assert_eq!(value["host_environment"]["FIXTURE_ALLOWED_TOKEN"], true);
    assert_eq!(value["host_environment"]["FIXTURE_ABSENT"], false);
}

#[test]
fn allowlist_rejects_execution_mutating_variables() {
    let (_t, mut p, _, _, _) = support::fixture("test -f input");
    for name in ["DYLD_INSERT_LIBRARIES", "RUSTC_WRAPPER", "RUSTFLAGS", "IFS"] {
        p.environment_allowlist = vec![name.into()];
        assert!(p.validate().is_err(), "execution binding accepted: {name}");
    }
}
#[test]
fn redaction_preserves_complete_output_and_masks_only_truncated_streams() {
    let status = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "redaction_child", "--ignored", "--nocapture"])
        .env("FIXTURE_ALLOWED_TOKEN", "secret-canary")
        .status()
        .unwrap();
    assert!(status.success());
}
#[tokio::test]
#[ignore = "private redaction environment subprocess"]
async fn redaction_child() {
    for (command, limit, stdout, stderr, overflow) in [
        ("test -f input && printf status", 128, "status", "", false),
        (
            "test -f input && printf '%s' \"$FIXTURE_ALLOWED_TOKEN\"",
            128,
            "[REDACTED]",
            "",
            false,
        ),
        (
            "test -f input && printf '%s' \"$FIXTURE_ALLOWED_TOKEN\" && printf status >&2",
            6,
            "[REDACTED]",
            "status",
            true,
        ),
        (
            "test -f input && printf status && printf '%s' \"$FIXTURE_ALLOWED_TOKEN\" >&2",
            6,
            "status",
            "[REDACTED]",
            true,
        ),
    ] {
        let (t, mut p, commit, c, refs) = support::fixture(command);
        p.environment_allowlist = vec!["FIXTURE_ALLOWED_TOKEN".into()];
        p.output_bytes = limit;
        let out = observe_commands(&p, &commit, &c, "chain", &refs, &t.path().join("evidence"))
            .await
            .unwrap();
        assert_eq!(out.checks[0].stdout, stdout.as_bytes());
        assert_eq!(out.checks[0].stderr, stderr.as_bytes());
        assert_eq!(out.checks[0].operational_error.is_some(), overflow);
    }
}
