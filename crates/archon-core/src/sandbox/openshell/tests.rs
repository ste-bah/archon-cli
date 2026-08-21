//! openshell.rs coverage, split out for the 500-line file-size gate.
//!
//! Declared with #[path] from openshell.rs, so super still means that
//! module and the assertions read exactly as they did in place.

use super::*;

#[test]
fn openshell_defaults_are_safe() {
    let cfg = OpenShellConfig::default();

    assert!(!cfg.enabled);
    assert_eq!(cfg.binary, "openshell");
    assert_eq!(cfg.workspace_mode, "upload");
    assert_eq!(cfg.gateway.as_deref(), Some("openshell"));
    assert!(!cfg.provider_injection);
    assert!(!cfg.host_shell_fallback);
}

#[test]
fn doctor_fails_closed_when_binary_missing() {
    let cfg = OpenShellConfig {
        enabled: true,
        ..OpenShellConfig::default()
    };

    let report = openshell_doctor_report(&cfg, OpenShellProbe::missing("not installed"));

    assert_eq!(report.status, OpenShellDoctorStatus::MissingBinary);
    assert!(render_openshell_doctor_report(&report).contains("missing-binary"));
}

#[test]
fn doctor_rejects_provider_injection_and_host_fallback() {
    let cfg = OpenShellConfig {
        enabled: true,
        provider_injection: true,
        host_shell_fallback: true,
        ..OpenShellConfig::default()
    };

    let report = openshell_doctor_report(&cfg, OpenShellProbe::found("openshell 1.0.0"));

    assert_eq!(report.status, OpenShellDoctorStatus::UnsafeConfig);
    assert!(render_openshell_doctor_report(&report).contains("unsafe-config"));
}

#[test]
fn doctor_reports_routed_execution_without_provider_injection() {
    let cfg = OpenShellConfig {
        enabled: true,
        providers: vec!["my-claude".into()],
        ..OpenShellConfig::default()
    };

    let report = openshell_doctor_report(&cfg, OpenShellProbe::found("openshell 1.2.3"));
    let body = render_openshell_doctor_report(&report);

    assert_eq!(report.status, OpenShellDoctorStatus::ReadyDetectOnly);
    assert!(body.contains("Bash routes through OpenShell"));
    assert!(body.contains("providers are ignored"));
    assert!(body.contains("Anthropic spoofing remains host-side"));
}

#[test]
fn backend_fails_closed_when_openshell_missing() {
    let backend = OpenShellSandboxBackend::new(OpenShellConfig {
        enabled: true,
        binary: "__definitely_missing_openshell__".into(),
        ..OpenShellConfig::default()
    });

    let error = backend
        .check(
            "Bash",
            archon_permissions::ToolCapability::EXECUTION,
            &serde_json::json!({}),
        )
        .unwrap_err();

    assert!(error.contains("__definitely_missing_openshell__"));
}

#[tokio::test]
async fn backend_execute_bash_runs_fail_closed_preflight() {
    let backend = OpenShellSandboxBackend::new(OpenShellConfig {
        enabled: true,
        binary: "__definitely_missing_openshell__".into(),
        ..OpenShellConfig::default()
    });
    let result = backend
        .execute_bash(SandboxCommandRequest {
            command: "echo no-host".into(),
            working_dir: std::path::PathBuf::from("."),
            timeout_ms: 1000,
            max_output_bytes: 1024,
            env: Vec::new(),
        })
        .await
        .unwrap();

    assert!(result.is_error);
    assert!(result.content.contains("refused execution"));
    assert!(result.content.contains("__definitely_missing_openshell__"));
    assert!(result.content.contains("no host shell fallback"));
}

/// #201 Phase 6. The refusal is the feature: this backend builds a
/// throwaway sandbox per command, so a terminal has nowhere to live, and a
/// host shell offered in its place would run outside the boundary `Bash` is
/// being held to.
#[test]
fn a_terminal_is_refused_with_the_reason_rather_than_opened_on_the_host() {
    let backend = OpenShellSandboxBackend::new(OpenShellConfig {
        enabled: true,
        ..OpenShellConfig::default()
    });

    let SandboxTerminal::Refused(reason) = backend.terminal(&SandboxTerminalRequest {
        shell: None,
        workspace: std::path::PathBuf::from("/repo"),
        cwd: std::path::PathBuf::from("/repo"),
    }) else {
        panic!("openshell has no session for a terminal to live in");
    };

    assert!(reason.contains("throwaway sandbox per command"), "{reason}");
    assert!(
        reason.contains("docker"),
        "a refusal has to say what would work: {reason}"
    );
}
