use super::*;

const PREFIX: &str = "command::workflow_host_command_tests::environment_tests::";

fn isolated(test: &str) {
    let status = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--ignored",
            "--exact",
            &format!("{PREFIX}{test}"),
            "--nocapture",
        ])
        .env("PATH", std::env::var_os("PATH").unwrap_or_default())
        .env("HOME", std::env::temp_dir())
        .env("XDG_CONFIG_HOME", std::env::temp_dir().join("config"))
        .env("ARCHON_274_ALLOWED_TOKEN", "allowed-canary")
        .env("ARCHON_274_PRIVATE", "private-canary")
        .env("NODE_OPTIONS", "--require=/nonexistent-282-preload.js")
        .status()
        .unwrap();
    assert!(status.success(), "isolated {test} failed");
}

#[test]
fn freeze_request_environment_regression() {
    isolated("freeze_request_environment_child");
}

#[test]
fn allowlisted_request_environment_regression() {
    isolated("allowlisted_request_environment_child");
}

#[test]
fn execution_control_never_forwarded_regression() {
    isolated("execution_control_never_forwarded_child");
}

/// Issue 282: a context that names an execution-control variable (config
/// load refuses one, but a context is built in more than one place) still
/// never hands its value to a child; data names next to it still pass.
#[test]
#[ignore = "isolated process environment"]
fn execution_control_never_forwarded_child() {
    let temp = tempfile::tempdir().unwrap();
    let mut context = configured_context(temp.path());
    context
        .acceptance_environment_allowlist
        .push("NODE_OPTIONS".into());
    let resolved = resolve(&context);
    assert!(
        !resolved.environment.contains_key("NODE_OPTIONS"),
        "execution control forwarded"
    );
    assert!(
        resolved
            .environment
            .contains_key("ARCHON_274_ALLOWED_TOKEN")
    );
}

#[test]
fn none_profile_environment_regression() {
    isolated("none_profile_environment_child");
}

#[test]
fn supervisor_environment_regression() {
    isolated("supervisor_environment_child");
}

#[cfg(any(unix, windows))]
#[test]
fn non_unicode_path_environment_regression() {
    #[cfg(unix)]
    let path = {
        use std::os::unix::ffi::OsStringExt;
        std::ffi::OsString::from_vec(b"/path-with-\xff".to_vec())
    };
    #[cfg(windows)]
    let path = {
        use std::os::windows::ffi::OsStringExt;
        std::ffi::OsString::from_wide(&[0xD800])
    };
    let status = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--ignored",
            "--exact",
            &format!("{PREFIX}non_unicode_path_child"),
            "--nocapture",
        ])
        .env("PATH", path)
        .status()
        .unwrap();
    assert!(status.success());
}

#[cfg(any(unix, windows))]
#[test]
#[ignore = "isolated process environment"]
fn non_unicode_path_child() {
    let temp = tempfile::tempdir().unwrap();
    let resolved = resolve(&configured_context(temp.path()));
    assert_eq!(
        std::ffi::OsStr::new(
            resolved
                .environment
                .get("PATH")
                .expect("request missing non-Unicode PATH")
        ),
        std::env::var_os("PATH").unwrap()
    );
}

fn configured_context(root: &std::path::Path) -> HostCommandResolutionContext {
    let mut context = context(root);
    // These are the names from the merged acceptance execution configuration,
    // carried by the launch and resume resolution contexts, never the values.
    let policy: archon_core::config::AcceptanceExecutionConfig =
        serde_json::from_value(serde_json::json!({
            "repository": root, "scratch_parent": root.join("scratch"), "project_inputs": [],
            "project_repository_view": "separate", "toolchain_path": root,
            "environment_allowlist": ["ARCHON_274_ALLOWED_TOKEN"], "cargo_seed": null,
            "timeout_secs": 5, "output_bytes": 1024, "scratch_bytes": 1024
        }))
        .unwrap();
    context.acceptance_environment_allowlist = policy.environment_allowlist;
    context
}

fn resolve(
    context: &HostCommandResolutionContext,
) -> super::super::workflow_host_command_catalog::ResolvedHostCommand {
    let catalog = fixed_decomposition_catalog("rev-1").unwrap();
    let request = HostCommandRequest::new("freeze-acceptance", Some("{}".into())).unwrap();
    resolve_host_command(&request, &catalog, context, "call-1").unwrap()
}

#[test]
#[ignore = "isolated process environment"]
fn freeze_request_environment_child() {
    let temp = tempfile::tempdir().unwrap();
    let resolved = resolve(&configured_context(temp.path()));
    for name in ["PATH", "HOME", "ARCHON_274_ALLOWED_TOKEN"] {
        assert!(
            resolved.environment.contains_key(name),
            "request missing {name}"
        );
    }
    assert!(!resolved.environment.contains_key("ARCHON_274_PRIVATE"));
}

#[test]
#[ignore = "isolated process environment"]
fn allowlisted_request_environment_child() {
    let temp = tempfile::tempdir().unwrap();
    let resolved = resolve(&configured_context(temp.path()));
    assert_eq!(
        resolved
            .environment
            .get("ARCHON_274_ALLOWED_TOKEN")
            .expect("request missing ARCHON_274_ALLOWED_TOKEN"),
        "allowed-canary"
    );
}

#[test]
#[ignore = "isolated process environment"]
fn none_profile_environment_child() {
    let temp = tempfile::tempdir().unwrap();
    let context = configured_context(temp.path());
    let catalog = fixed_decomposition_catalog("rev-1").unwrap();
    let none = [
        "freeze-skeleton",
        "requirements-trace",
        "verify-frozen-acceptance",
        "verify-frozen-skeleton",
    ];
    // These are exactly the catalog's commands that run no project code.
    for (id, capability) in &catalog.capabilities {
        assert_eq!(
            capability.environment_profile == archon_workflow::EnvironmentProfileId::None,
            none.contains(&id.as_str()),
            "{id} profile"
        );
    }
    for id in none {
        let stdin = (id == "freeze-skeleton").then(|| "{}".into());
        let request = HostCommandRequest::new(id, stdin).unwrap();
        let resolved = resolve_host_command(&request, &catalog, &context, "call-1").unwrap();
        assert!(
            resolved.environment.contains_key("PATH"),
            "{id} missing PATH"
        );
        assert!(
            resolved.environment.contains_key("HOME"),
            "{id} missing HOME"
        );
        assert!(!resolved.environment.contains_key("ARCHON_274_PRIVATE"));
        assert!(
            !resolved
                .environment
                .contains_key("ARCHON_274_ALLOWED_TOKEN"),
            "{id} received an allowlisted value"
        );
    }
}

#[tokio::test]
#[ignore = "isolated process environment"]
async fn supervisor_environment_child() {
    let temp = tempfile::tempdir().unwrap();
    let mut resolved = resolve(&configured_context(temp.path()));
    resolved.program = std::env::current_exe().unwrap();
    resolved.args = vec![
        "--ignored".into(),
        "--exact".into(),
        format!("{PREFIX}supervised_environment_reader"),
        "--nocapture".into(),
    ];
    resolved.stdin = None;
    let (control, _handle) =
        super::super::workflow_host_command_supervisor::HostCommandControl::new();
    let output = super::super::workflow_host_command_supervisor::supervise_process_group(
        resolved, control, None,
    )
    .await
    .unwrap();
    assert_eq!(
        output.exit_code,
        Some(0),
        "stdout: {}; stderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
#[ignore = "supervisor child process"]
fn supervised_environment_reader() {
    for name in ["PATH", "HOME"] {
        assert!(std::env::var_os(name).is_some(), "child missing {name}");
    }
    #[cfg(unix)]
    assert!(
        std::env::var_os("XDG_CONFIG_HOME").is_some(),
        "child missing XDG_CONFIG_HOME"
    );
    assert_eq!(
        std::env::var("ARCHON_274_ALLOWED_TOKEN").unwrap(),
        "allowed-canary"
    );
    assert!(std::env::var_os("ARCHON_274_PRIVATE").is_none());
    #[cfg(windows)]
    for name in [
        "SystemRoot",
        "USERPROFILE",
        "PATHEXT",
        "TEMP",
        "TMP",
        "COMSPEC",
    ] {
        assert!(
            std::env::var_os(name).is_some(),
            "Windows child missing {name}"
        );
    }
}

#[test]
fn catalog_digest_excludes_runtime_environment_values() {
    isolated("catalog_digest_environment_child");
}

#[test]
#[ignore = "isolated process environment"]
fn catalog_digest_environment_child() {
    use super::super::{workflow_decompose, workflow_decompose_identity, workflow_provider_route};
    let temp = tempfile::tempdir().unwrap();
    let context = configured_context(temp.path());
    let catalog = fixed_decomposition_catalog("rev-1").unwrap();
    let request = HostCommandRequest::new("freeze-acceptance", Some("{}".into())).unwrap();
    let identity = archon_workflow::FixedRunIdentityV1 {
        template_version: "fixed-decomposition-v1".into(),
        starting_binary_revision: "rev-1".into(),
        script_digest: "script".into(),
        catalog_digest: catalog.digest.clone(),
        project_root_identity: "project".into(),
        prd_identity: "prd".into(),
        task_root_identity: "tasks".into(),
    };
    let route = workflow_provider_route::resolve_anthropic_route(
        None,
        workflow_provider_route::ProviderEndpointPolicy::ConfiguredOnly,
    );
    let arguments = serde_json::json!({"projectRoot": "project"});
    let launch_digest =
        workflow_decompose::fixed_launch_digest(&identity, &arguments, &catalog, &route).unwrap();
    let decomposition_identity =
        workflow_decompose_identity::fixed_decomposition_identity().unwrap();
    let tokens = super::super::workflow_host_command_catalog::host_command_identity_tokens(
        &context,
        "freeze-acceptance",
    )
    .unwrap();
    let call_id = archon_workflow::host_command_call_id(
        "freeze-acceptance",
        &catalog.digest,
        "rev-1",
        &tokens,
        b"{}",
    );
    for value in ["first-secret", "different-secret"] {
        for name in ["PATH", "HOME", "ARCHON_274_ALLOWED_TOKEN"] {
            // Only this exact test runs in this subprocess; no parent env is mutated.
            unsafe { std::env::set_var(name, value) };
        }
        let resolved = resolve_host_command(&request, &catalog, &context, "call-1").unwrap();
        for name in ["PATH", "HOME", "ARCHON_274_ALLOWED_TOKEN"] {
            assert_eq!(
                resolved
                    .environment
                    .get(name)
                    .unwrap_or_else(|| panic!("request missing {name}")),
                value,
                "runtime value missing: {name}"
            );
        }
        let current = fixed_decomposition_catalog("rev-1").unwrap();
        assert_eq!(catalog.digest, current.digest);
        assert_eq!(
            launch_digest,
            workflow_decompose::fixed_launch_digest(&identity, &arguments, &current, &route)
                .unwrap()
        );
        assert_eq!(
            decomposition_identity,
            workflow_decompose_identity::fixed_decomposition_identity().unwrap()
        );
        let resumed_identity = archon_workflow::FixedRunIdentityV1 {
            catalog_digest: current.digest.clone(),
            ..identity.clone()
        };
        archon_workflow::verify_fixed_resume_identity(&identity, &resumed_identity).unwrap();
        let current_tokens =
            super::super::workflow_host_command_catalog::host_command_identity_tokens(
                &context,
                "freeze-acceptance",
            )
            .unwrap();
        assert_eq!(
            call_id,
            archon_workflow::host_command_call_id(
                "freeze-acceptance",
                &current.digest,
                "rev-1",
                &current_tokens,
                b"{}"
            )
        );
        assert!(!serde_json::to_string(&current).unwrap().contains(value));
    }
}
