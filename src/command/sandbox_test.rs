use anyhow::Result;

pub(super) fn render_test(
    config: &archon_core::sandbox::SandboxConfig,
    backend: Option<String>,
) -> Result<String> {
    let mut policy = config.policy().map_err(anyhow::Error::msg)?;
    if let Some(backend) = backend {
        policy.backend = backend.parse().map_err(anyhow::Error::msg)?;
    }
    policy.validate().map_err(anyhow::Error::msg)?;
    let report = test_report(config, &policy);
    Ok(format!(
        "Sandbox test\nBackend: {}\nConfig: valid\nExecution: {}\nReason: {}\nNo untrusted command was run\n",
        policy.backend, report.execution, report.reason
    ))
}

struct SandboxTestReport {
    execution: &'static str,
    reason: String,
}

fn test_report(
    config: &archon_core::sandbox::SandboxConfig,
    policy: &archon_core::sandbox::SandboxPolicy,
) -> SandboxTestReport {
    match policy.backend {
        archon_core::sandbox::SandboxBackendKind::Docker => docker_report(&config.docker),
        archon_core::sandbox::SandboxBackendKind::Ssh => ssh_report(&config.ssh),
        archon_core::sandbox::SandboxBackendKind::OpenShell => openshell_report(&config.openshell),
        archon_core::sandbox::SandboxBackendKind::Logical => SandboxTestReport {
            execution: "available-logical",
            reason: "logical gate only; no process, filesystem, or network isolation".into(),
        },
        archon_core::sandbox::SandboxBackendKind::Disabled => SandboxTestReport {
            execution: "unavailable",
            reason: "sandbox backend disabled; normal permission checks still apply".into(),
        },
    }
}

fn docker_report(config: &archon_core::sandbox::DockerConfig) -> SandboxTestReport {
    let report = archon_core::sandbox::docker_doctor_report(
        config,
        archon_core::sandbox::probe_docker(&config.binary),
    );
    match report.status {
        archon_core::sandbox::DockerDoctorStatus::ReadyDetectOnly => SandboxTestReport {
            execution: "available-detect-only",
            reason: format!(
                "docker binary reachable; image={} network={}",
                config.image, config.network
            ),
        },
        _ => SandboxTestReport {
            execution: "unavailable-detect-only",
            reason: report.findings.join("; "),
        },
    }
}

fn ssh_report(config: &archon_core::sandbox::SshConfig) -> SandboxTestReport {
    let report = archon_core::sandbox::ssh_doctor_report(
        config,
        archon_core::sandbox::probe_ssh(&config.binary),
    );
    match report.status {
        archon_core::sandbox::SshDoctorStatus::ReadyDetectOnly => SandboxTestReport {
            execution: "available-detect-only",
            reason: "ssh binary reachable and remote routing policy is valid".into(),
        },
        _ => SandboxTestReport {
            execution: "unavailable-detect-only",
            reason: report.findings.join("; "),
        },
    }
}

fn openshell_report(config: &archon_core::sandbox::OpenShellConfig) -> SandboxTestReport {
    let report = archon_core::sandbox::openshell_doctor_report(
        config,
        archon_core::sandbox::probe_openshell(&config.binary),
    );
    match report.status {
        archon_core::sandbox::OpenShellDoctorStatus::ReadyDetectOnly => SandboxTestReport {
            execution: "available-detect-only",
            reason: "openshell binary reachable and mediated execution policy is valid".into(),
        },
        _ => SandboxTestReport {
            execution: "unavailable-detect-only",
            reason: report.findings.join("; "),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn disabled_backend_reports_unavailable_without_execution() {
        let body = render_test(
            &archon_core::sandbox::SandboxConfig::default(),
            Some("openshell".into()),
        )
        .unwrap();

        assert!(body.contains("Backend: openshell"));
        assert!(body.contains("unavailable-detect-only"));
        assert!(body.contains("No untrusted command was run"));
    }

    #[test]
    fn logical_backend_reports_policy_gate_only() {
        let config = archon_core::sandbox::SandboxConfig {
            backend: "logical".into(),
            ..archon_core::sandbox::SandboxConfig::default()
        };

        let body = render_test(&config, None).unwrap();

        assert!(body.contains("available-logical"));
        assert!(body.contains("logical gate only"));
    }
}
