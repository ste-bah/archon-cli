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
        .env("CARGO_BUILD_JOBS", "2")
        .env("RUST_TEST_THREADS", "4")
        .env("HOME", root.path())
        .env("JAVA_HOME", root.path())
        .env("LANG", "C")
        .env("OPERATOR_SECRET", "hidden")
        .env("FIXTURE_API_KEY", "hidden-data")
        .env("HTTPS_PROXY", "https://user:password@proxy.invalid");
    if path {
        child.env("PATH", std::env::var_os("PATH").unwrap());
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

macro_rules! verdict_case {
    ($test:ident, $script:literal) => {
        #[test]
        fn $test() {
            operator(stringify!($test), true, |root| {
                assert!(matches!(
                    run_script(root, $script),
                    Ran::Finished {
                        code: Some(3),
                        success: false,
                        ..
                    }
                ));
            });
        }
    };
}
verdict_case!(
    review349_falsify_optional,
    "echo 'FIXTURE_API_KEY environment variable is optional'; exit 3"
);
verdict_case!(
    review349_falsify_affirmative,
    "echo 'environment variable FIXTURE_API_KEY is set'; exit 3"
);
verdict_case!(
    review349_falsify_quoted,
    "echo \"expected 'FIXTURE_API_KEY environment variable is not set' in stderr\"; exit 3"
);

#[test]
fn review349_falsify_consumes_operator_config() {
    operator("review349_falsify_consumes_operator_config", true, |root| {
        let mut config = archon_core::config::ArchonConfig::default();
        config.workflow.acceptance_execution =
            Some(archon_core::config::AcceptanceExecutionConfig {
                repository: root.into(),
                scratch_parent: root.join("scratch"),
                project_inputs: vec![],
                project_input_excludes: vec![],
                project_repository_view: Default::default(),
                toolchain_path: "/usr/bin:/bin".into(),
                environment: Default::default(),
                environment_allowlist: vec!["FIXTURE_API_KEY".into()],
                cargo_seed: None,
                timeout_secs: 10,
                output_bytes: 1024,
                scratch_bytes: 1024,
                external_data_roots: vec![],
            });
        let policy = crate::command::requirement_trace::check_policy::from_config(&config).unwrap();
        let argv = vec![
            "/bin/sh".into(),
            "-c".into(),
            "test \"$FIXTURE_API_KEY\" = hidden-data && test -z \"${OPERATOR_SECRET-}\"".into(),
        ];
        assert!(matches!(
            run_with_policy(root, &argv, Duration::from_secs(5), policy.as_ref()),
            Ran::Finished { success: true, .. }
        ));
        let mut missing = policy.unwrap();
        missing.forwarded = vec!["ABSENT_API_KEY".into()];
        assert!(matches!(
            run_with_policy(
                root,
                &["/bin/sh".into(), "-c".into(), "touch launched".into()],
                Duration::from_secs(5),
                Some(&missing)
            ),
            Ran::NotLaunchable { .. }
        ));
        assert!(!root.join("launched").exists());
        let configured = config.workflow.acceptance_execution.as_mut().unwrap();
        configured
            .environment
            .insert("FIXTURE_API_KEY".into(), "literal-secret".into());
        let error = crate::command::requirement_trace::check_policy::from_config(&config)
            .unwrap_err()
            .to_string();
        assert!(
            error.contains("FIXTURE_API_KEY") && !error.contains("literal-secret"),
            "{error}"
        );
        config
            .workflow
            .acceptance_execution
            .as_mut()
            .unwrap()
            .environment
            .clear();
        config
            .workflow
            .acceptance_execution
            .as_mut()
            .unwrap()
            .toolchain_path = "relative/toolchain".into();
        assert!(crate::command::requirement_trace::check_policy::from_config(&config).is_err());
        missing.forwarded = vec!["BASH_ENV".into()];
        assert!(matches!(
            run_with_policy(root, &argv, Duration::from_secs(5), Some(&missing)),
            Ran::NotLaunchable { .. }
        ));
    });
}

verdict_case!(
    r3_falsify_setup_success,
    "echo 'setup: set FIXTURE_API_KEY successfully'; exit 3"
);
verdict_case!(
    r3_falsify_export_success,
    "echo 'setup: export FIXTURE_API_KEY successfully'; exit 3"
);
verdict_case!(
    r3_falsify_escaped_expectation,
    r#"printf '%s\n' 'expected "error \"FIXTURE_API_KEY environment variable is not set\"" in stderr'; exit 3"#
);

macro_rules! missing_case {
    ($id:ident, $script:literal) => {
        #[test]
        fn $id() {
            operator(stringify!($id), true, |root| {
                match run_script(root, $script) {
                    Ran::NotLaunchable { reason } => assert!(reason.contains("FIXTURE_API_KEY")),
                    _ => panic!("a missing host variable became a mutant kill"),
                }
            });
        }
    };
}
missing_case!(
    r3_falsify_rust_err_string,
    "echo 'called Result::unwrap() on an Err value: \"FIXTURE_API_KEY environment variable is not set\"'; exit 3"
);
missing_case!(
    r3_falsify_json_error,
    "echo '{\"error\":\"FIXTURE_API_KEY environment variable is not set\"}'; exit 3"
);
missing_case!(
    r3_falsify_json_message,
    "echo '{\"message\":\"missing environment variable FIXTURE_API_KEY\"}'; exit 3"
);

macro_rules! noted_case {
    ($id:ident, $message:literal) => {
        #[test]
        fn $id() {
            operator(stringify!($id), true, |root| {
                let script = format!("printf '%s\\n' '{}'; exit 3", $message);
                match run_script(root, &script) {
                    Ran::Finished {
                        code: Some(3),
                        success: false,
                        output,
                    } => {
                        assert!(
                            output.contains("withheld variable")
                                && output.contains("FIXTURE_API_KEY"),
                            "ambiguous evidence lacked a visible note: {output}"
                        );
                    }
                    _ => panic!("ambiguous evidence changed the verifier result"),
                }
            });
        }
    };
}
noted_case!(
    r3_falsify_note_optional,
    "FIXTURE_API_KEY environment variable is optional"
);
noted_case!(
    r3_falsify_note_affirmative,
    "environment variable FIXTURE_API_KEY is set"
);
noted_case!(
    r3_falsify_note_unknown_quote,
    "actual text: \"FIXTURE_API_KEY is not set\""
);
