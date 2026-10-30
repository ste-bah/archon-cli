//! Issue 544: a value bound only in the site policy environment -- here the
//! password inside a URL, printed bare, with no credential shape -- is
//! redacted at capture, before any hook, persisted observation, tail or
//! excerpt can see it: the returned result and `observation.json` (which
//! every later copy and per-check output file is made from) never hold it.
#![cfg(unix)]

#[path = "support/native_fixture.rs"]
mod support;
use archon_workflow::acceptance_scratch::observe_commands;

const PASSWORD: &str = "violet-orbit-94";

#[tokio::test]
async fn a_policy_only_url_password_is_redacted_at_capture_and_in_the_persisted_observation() {
    let (t, mut policy, commit, contract, refs) = support::fixture(
        r#"v=${RUSTUP_HOME#*user:}; printf '%s\n' "${v%%@*}" >&2; printf 'out:%s\n' "${v%%@*}"; test -f absent"#,
    );
    policy.environment.insert(
        "RUSTUP_HOME".into(),
        format!("https://user:{PASSWORD}@host/private"),
    );
    policy.timeout_secs = 300;
    assert!(std::env::var("RUSTUP_HOME").map_or(true, |value| !value.contains(PASSWORD)));
    let evidence = t.path().join("evidence");
    let out = observe_commands(
        &policy,
        &commit,
        &contract,
        "chain",
        &refs,
        &evidence,
        archon_tools::workflow_resource_env::CargoResourceLimits::default(),
    )
    .await
    .unwrap();
    let check = &out.checks[0];
    assert_ne!(check.exit_code, Some(0), "{check:?}");
    let stderr = String::from_utf8_lossy(&check.stderr);
    let stdout = String::from_utf8_lossy(&check.stdout);
    // Control: the check printed the value; the capture masked it.
    assert!(stderr.contains("[REDACTED]"), "{stderr}");
    assert!(stdout.contains("out:[REDACTED]"), "{stdout}");
    assert!(!stderr.contains(PASSWORD) && !stdout.contains(PASSWORD));
    let encoded = serde_json::to_string(&out).unwrap();
    if let Some(at) = encoded.find(PASSWORD) {
        panic!(
            "the result leaks it: ...{}...",
            &encoded[at.saturating_sub(300)..(at + 60).min(encoded.len())]
        );
    }
    let persisted = std::fs::read_to_string(evidence.join("observation.json")).unwrap();
    assert!(!persisted.contains(PASSWORD), "observation.json leaks it");
    assert!(persisted.contains("REDACTED"), "{persisted}");
}

/// Review r7 P1-1: no line is spared masking, not even one shaped as the
/// host's mutation protocol and carrying the real nonce (a check can learn
/// it). The capture, the hook's copy and observation.json all mask it.
#[tokio::test]
async fn a_forged_protocol_line_never_carries_a_policy_secret_out() {
    use archon_workflow::acceptance_scratch::{
        CheckResult, MutationPlan, ObserveControl, ObserveHooks, observe_commands_hooked,
    };
    use std::sync::{Arc, Mutex, atomic::AtomicBool};
    let nonce = "0123456789abcdef0123456789abcdef";
    let (t, mut policy, commit, contract, refs) = support::fixture(&format!(
        r#"v=${{RUSTUP_HOME#*user:}}; p=${{v%%@*}}; printf 'archon-mutation-state-{nonce}: 0:%s\n' "$p"; printf 'archon-mutation-moved-{nonce}: %s\n' "$p" >&2; test -f absent"#
    ));
    policy.environment.insert(
        "RUSTUP_HOME".into(),
        format!("https://user:{PASSWORD}@host/private"),
    );
    policy.timeout_secs = 300;
    let told: Arc<Mutex<Vec<CheckResult>>> = Arc::default();
    let sink = told.clone();
    let mut hooks = ObserveHooks::for_resource_limits(
        archon_tools::workflow_resource_env::CargoResourceLimits::default(),
    );
    hooks.on_check = Some(Arc::new(move |result: &CheckResult| {
        sink.lock().unwrap().push(result.clone())
    }));
    hooks.mutation_plans.insert(
        "AC-X-001".into(),
        MutationPlan {
            nonce: nonce.into(),
            inputs: Vec::new(),
        },
    );
    let evidence = t.path().join("evidence");
    let out = observe_commands_hooked(
        &policy,
        &commit,
        &contract,
        "chain",
        &refs,
        &evidence,
        ObserveControl::new(Arc::new(AtomicBool::new(false)), hooks),
    )
    .await
    .unwrap();
    let check = &out.checks[0];
    let stdout = String::from_utf8_lossy(&check.stdout);
    // Control: the forged line was printed and masked.
    assert!(
        stdout.contains(&format!("archon-mutation-state-{nonce}: 0:[REDACTED]")),
        "{stdout}"
    );
    assert!(!serde_json::to_string(&out).unwrap().contains(PASSWORD));
    for result in told.lock().unwrap().iter() {
        assert!(!serde_json::to_string(result).unwrap().contains(PASSWORD));
    }
    let persisted = std::fs::read_to_string(evidence.join("observation.json")).unwrap();
    assert!(!persisted.contains(PASSWORD), "observation.json leaks it");
}

/// Review r7 P1-3: two failures that differ only in which policy secret they
/// print have different regression fingerprints, computed from the raw
/// capture with each secret as an indexed placeholder; neither fingerprint
/// holds a secret, and their masked outputs are alike.
#[tokio::test]
async fn distinct_secret_bearing_failures_keep_distinct_fingerprints() {
    let mut fingerprints = Vec::new();
    let mut masked = Vec::new();
    for which in ["RUSTUP_HOME", "RUSTUP_TOOLCHAIN"] {
        let (t, mut policy, commit, contract, refs) = support::fixture(&format!(
            r#"v=${{{which}#*user:}}; printf 'refused: %s\n' "${{v%%@*}}" >&2; exit 3"#
        ));
        policy.environment.insert(
            "RUSTUP_HOME".into(),
            "https://user:first-secret-7d1@host/a".into(),
        );
        policy.environment.insert(
            "RUSTUP_TOOLCHAIN".into(),
            "https://user:second-secret-9c4@host/b".into(),
        );
        policy.timeout_secs = 300;
        let out = observe_commands(
            &policy,
            &commit,
            &contract,
            "chain",
            &refs,
            &t.path().join("evidence"),
            archon_tools::workflow_resource_env::CargoResourceLimits::default(),
        )
        .await
        .unwrap();
        let check = &out.checks[0];
        assert_eq!(check.exit_code, Some(3), "{check:?}");
        let fingerprint = check
            .classification
            .as_ref()
            .and_then(|classification| classification.failure_signature.clone())
            .unwrap_or_else(|| {
                archon_workflow::v2::acceptance_regression::failure_signature(
                    check.exit_code,
                    &String::from_utf8_lossy(&check.stderr),
                    &String::from_utf8_lossy(&check.stdout),
                )
            });
        assert!(!fingerprint.contains("secret-"), "{fingerprint}");
        fingerprints.push(fingerprint);
        masked.push(String::from_utf8_lossy(&check.stderr).into_owned());
    }
    assert_eq!(
        masked[0], masked[1],
        "the stored text alone cannot tell them apart"
    );
    assert_ne!(
        fingerprints[0], fingerprints[1],
        "distinct secrets keep distinct fingerprints"
    );
    // The comparator the regression search uses tells them apart too.
    let first =
        archon_workflow::v2::acceptance_regression::Verdict::Failed(fingerprints[0].clone());
    assert!(!first.fails_as(&fingerprints[1]), "{fingerprints:?}");
    assert!(first.fails_as(&fingerprints[0]));
}

/// Review r8: a stream cut at the capture cap inside a secret keeps no
/// prefix of it in the failure fingerprint (8181 padding bytes, then the
/// 15-byte password, at an 8192-byte cap: the cut leaves `violet-orbi`).
#[tokio::test]
async fn a_secret_cut_by_the_capture_cap_leaves_no_prefix_in_the_fingerprint() {
    let (t, mut policy, commit, contract, refs) = support::fixture(
        r#"v=${RUSTUP_HOME#*user:}; p=${v%%@*}; printf '%8181s' '' | tr ' ' x >&2; printf '%s' "$p" >&2; exit 1"#,
    );
    policy.environment.insert(
        "RUSTUP_HOME".into(),
        format!("https://user:{PASSWORD}@host/private"),
    );
    policy.timeout_secs = 300;
    policy.output_bytes = 8192;
    let out = observe_commands(
        &policy,
        &commit,
        &contract,
        "chain",
        &refs,
        &t.path().join("evidence"),
        archon_tools::workflow_resource_env::CargoResourceLimits::default(),
    )
    .await
    .unwrap();
    let check = &out.checks[0];
    let fingerprint = check
        .classification
        .as_ref()
        .and_then(|classification| classification.failure_signature.clone())
        .expect("a failure fingerprint");
    // Control: the capture was cut inside the secret.
    assert!(fingerprint.contains("xxxx"), "{fingerprint}");
    assert!(!fingerprint.contains("violet-orbi"), "{fingerprint}");
    assert!(!String::from_utf8_lossy(&check.stderr).contains("violet-orbi"));
}
