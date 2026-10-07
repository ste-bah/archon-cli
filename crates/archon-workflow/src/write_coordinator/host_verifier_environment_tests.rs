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
    if case.starts_with("review349_cache_") {
        for key in archon_tools::build_cache_env::toolchain_cache_env_keys() {
            child.env(key, root.path().join("cold-default"));
        }
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

struct CacheDispatch {
    vars: Vec<(String, String)>,
}
#[async_trait::async_trait]
impl crate::agent_dispatch_port::WorkflowAgentDispatch for CacheDispatch {
    fn fanout_parallelism(&self, _: Option<usize>) -> usize {
        1
    }
    async fn host_command_env(&self, _: &Path) -> crate::agent_dispatch_port::HostCommandEnv {
        crate::agent_dispatch_port::HostCommandEnv {
            vars: self.vars.clone(),
            hold: None,
        }
    }
    async fn run_call(
        &self,
        _: &str,
        _: Option<String>,
        _: &crate::v2::WorkflowV2CallExecution,
        _: &crate::v2::WorkflowV2AgentAdapter,
        _: Option<&crate::v2::WorkflowV2ResultStore>,
        _: Option<&crate::task_universe::WorkflowV2TaskUniverse>,
    ) -> crate::WorkflowResult<crate::v2::WorkflowV2Result> {
        panic!("no agent calls")
    }
}

macro_rules! cache_case {
    ($test:ident, $marker:literal, $key:literal) => {
        #[test]
        fn $test() {
            operator(stringify!($test), true, |root| {
                std::fs::write(root.join($marker), "").unwrap();
                let lease = root.join("leased");
                let vars = archon_tools::build_cache_env::cache_env_for_repository(root, &lease, &[]);
                let expected = vars.iter().find(|(name, _)| name == $key).unwrap().1.clone();
                std::fs::create_dir_all(&expected).unwrap();
                std::fs::write(Path::new(&expected).join("offline-fixture"), "cached").unwrap();
                let mut dispatch = CacheDispatch { vars };
                dispatch.vars.push(("OPERATOR_SECRET".into(), "must-be-filtered".into()));
                let script = format!("test -f \"${{{}}}/offline-fixture\" && test -z \"${{OPERATOR_SECRET-}}${{FIXTURE_API_KEY-}}\"", $key);
                let result = tokio::runtime::Runtime::new().unwrap().block_on(
                    crate::v2::write::test_baseline_run::run_in_worktree(&dispatch, root, &script, None));
                assert_eq!(result.exit_code, Some(0), "{result:?}");
                assert!(result.error.is_none(), "{result:?}");
            });
        }
    }
}
cache_case!(review349_cache_go, "go.mod", "GOCACHE");
cache_case!(review349_cache_npm, "package.json", "npm_config_cache");
cache_case!(review349_cache_yarn, "package.json", "YARN_CACHE_FOLDER");
cache_case!(review349_cache_pip, "requirements.txt", "PIP_CACHE_DIR");
cache_case!(review349_cache_uv, "pyproject.toml", "UV_CACHE_DIR");
cache_case!(review349_cache_maven, "pom.xml", "MAVEN_OPTS_LOCAL_REPO");

#[test]
fn review349_recorded_policy_reaches_host_verifiers() {
    operator(
        "review349_recorded_policy_reaches_host_verifiers",
        true,
        |root| {
            let run = root.join("run");
            crate::write_coordinator::project_inputs::write_test_policy(&run, root, &[]);
            let file = run.join("v2/generated-metadata.json");
            let mut metadata: serde_json::Value =
                serde_json::from_slice(&std::fs::read(&file).unwrap()).unwrap();
            metadata["observer_snapshot"]["native_execution"]["policy"]["toolchain_path"] =
                serde_json::json!("/usr/bin:/bin");
            metadata["observer_snapshot"]["native_execution"]["policy"]["environment_allowlist"] =
                serde_json::json!(["FIXTURE_API_KEY"]);
            std::fs::write(&file, serde_json::to_vec(&metadata).unwrap()).unwrap();
            let script = "test \"$FIXTURE_API_KEY\" = hidden-data && test -z \"${OPERATOR_SECRET-}${HTTPS_PROXY-}\"";
            unbounded_for_tests(true);
            let (mut child, boundary, _) = command(Path::new("/bin/sh"), Some(&run), &[]).unwrap();
            unbounded_for_tests(false);
            assert!(
                child
                    .args(["-c", script])
                    .output()
                    .unwrap()
                    .status
                    .success()
            );
            boundary.finish("configured verifier").unwrap();
            let dispatch = CacheDispatch { vars: vec![] };
            let result = tokio::runtime::Runtime::new().unwrap().block_on(
                crate::v2::write::test_baseline_run::run_in_worktree(
                    &dispatch,
                    root,
                    script,
                    Some(&run),
                ),
            );
            assert_eq!(result.exit_code, Some(0), "{result:?}");
        },
    );
}

fn stage_note_case(case: &str, message: &str) {
    operator(case, true, |root| {
        let command = format!("printf '%s\\n' '{}'; exit 3", message);
        let report = crate::acceptance::run_verify_command_capture(root, Some(&command), None)
            .expect("failure is still a captured result")
            .unwrap();
        assert_eq!(report.exit_code, Some(3));
        assert!(report.stderr.contains("Note:") && report.stderr.contains("FIXTURE_API_KEY"));
        let run = root.join("wave-run");
        let error = crate::write_coordinator::patch_apply::run_wave_verify(
            root,
            Some(&command),
            0,
            &run,
            "stage",
        )
        .unwrap_err();
        assert!(
            matches!(
                error,
                crate::write_coordinator::ApplyError::VerifyFailed { exit: 3, .. }
            ),
            "{error}"
        );
        let persisted =
            std::fs::read_to_string(run.join("write-coordination/stages/stage/tests/0.json"))
                .unwrap();
        assert!(persisted.contains("Note:") && persisted.contains("FIXTURE_API_KEY"));
        assert!(!persisted.contains("hidden-data"));
    });
}
#[test]
fn r4_stage_pytest_expectation() {
    stage_note_case(
        "r4_stage_pytest_expectation",
        "E AssertionError: expected FIXTURE_API_KEY environment variable is not set in stderr",
    );
}
#[test]
fn r4_stage_actual_diagnostic() {
    stage_note_case(
        "r4_stage_actual_diagnostic",
        "FIXTURE_API_KEY environment variable is not set",
    );
}
#[test]
fn r4_stage_actual_after_expectation() {
    stage_note_case(
        "r4_stage_actual_after_expectation",
        "E AssertionError: expected FIXTURE_API_KEY environment variable is not set in stderr\nFIXTURE_API_KEY environment variable is not set",
    );
}
