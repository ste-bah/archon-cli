pub(crate) type OpenShellDoctorOverride = (
    archon_core::sandbox::OpenShellConfig,
    archon_core::sandbox::OpenShellProbe,
);

pub(crate) type DockerDoctorOverride = (
    archon_core::sandbox::DockerConfig,
    archon_core::sandbox::DockerProbe,
);

pub(crate) type SshDoctorOverride = (
    archon_core::sandbox::SshConfig,
    archon_core::sandbox::SshProbe,
);

#[derive(Default)]
pub(crate) struct SandboxDoctorOverrides<'a> {
    pub(crate) docker: Option<&'a DockerDoctorOverride>,
    pub(crate) ssh: Option<&'a SshDoctorOverride>,
    pub(crate) openshell: Option<&'a OpenShellDoctorOverride>,
}

pub(crate) fn render_sandbox_doctor(
    args: &[String],
    overrides: SandboxDoctorOverrides<'_>,
) -> String {
    let backend = parse_backend_arg(args).unwrap_or("logical");
    match backend {
        "docker" => render_docker_doctor(overrides.docker),
        "ssh" => render_ssh_doctor(overrides.ssh),
        "openshell" => render_openshell_doctor(overrides.openshell),
        "logical" => {
            "Sandbox doctor\nBackend: logical\nStatus: available\nIsolation: policy gate only, not OS/container isolation\nExecution: host tool execution remains governed by permission preflight\n".into()
        }
        other => {
            format!("Sandbox doctor\nBackend: {other}\nStatus: not implemented in this build\n")
        }
    }
}

fn render_docker_doctor(docker_override: Option<&DockerDoctorOverride>) -> String {
    if let Some((config, probe)) = docker_override {
        let report = archon_core::sandbox::docker_doctor_report(config, probe.clone());
        return archon_core::sandbox::render_docker_doctor_report(&report);
    }

    let config = match load_sandbox_config_without_writing() {
        Ok(config) => config.docker,
        Err(error) => {
            return format!(
                "Sandbox doctor\nBackend: docker\nStatus: config-error\n- {error}\nExecution: disabled\n"
            );
        }
    };
    let probe = archon_core::sandbox::probe_docker(&config.binary);
    let report = archon_core::sandbox::docker_doctor_report(&config, probe);
    archon_core::sandbox::render_docker_doctor_report(&report)
}

fn render_ssh_doctor(ssh_override: Option<&SshDoctorOverride>) -> String {
    if let Some((config, probe)) = ssh_override {
        let report = archon_core::sandbox::ssh_doctor_report(config, probe.clone());
        return archon_core::sandbox::render_ssh_doctor_report(&report);
    }

    let config = match load_sandbox_config_without_writing() {
        Ok(config) => config.ssh,
        Err(error) => {
            return format!(
                "Sandbox doctor\nBackend: ssh\nStatus: config-error\n- {error}\nExecution: disabled\n"
            );
        }
    };
    let probe = archon_core::sandbox::probe_ssh(&config.binary);
    let report = archon_core::sandbox::ssh_doctor_report(&config, probe);
    archon_core::sandbox::render_ssh_doctor_report(&report)
}

fn render_openshell_doctor(openshell_override: Option<&OpenShellDoctorOverride>) -> String {
    if let Some((config, probe)) = openshell_override {
        let report = archon_core::sandbox::openshell_doctor_report(config, probe.clone());
        return archon_core::sandbox::render_openshell_doctor_report(&report);
    }

    let config = match load_sandbox_config_without_writing() {
        Ok(config) => config.openshell,
        Err(error) => {
            return format!(
                "Sandbox doctor\nBackend: openshell\nStatus: config-error\n- {error}\nExecution: disabled\n"
            );
        }
    };
    let probe = archon_core::sandbox::probe_openshell(&config.binary);
    let report = archon_core::sandbox::openshell_doctor_report(&config, probe);
    archon_core::sandbox::render_openshell_doctor_report(&report)
}

fn parse_backend_arg(args: &[String]) -> Option<&str> {
    let mut iter = args.iter().map(String::as_str);
    while let Some(arg) = iter.next() {
        match arg {
            "--backend" => return iter.next(),
            value if !value.starts_with('-') => return Some(value),
            _ => {}
        }
    }
    None
}

fn load_sandbox_config_without_writing() -> Result<archon_core::sandbox::SandboxConfig, String> {
    let path = archon_core::config::default_config_path();
    if !path.exists() {
        return Ok(archon_core::sandbox::SandboxConfig::default());
    }
    archon_core::config::load_config_if_exists(path)
        .map(|config| config.map(|config| config.sandbox).unwrap_or_default())
        .map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sandbox_doctor_docker_reports_detect_only_status() {
        let override_report = (
            archon_core::sandbox::DockerConfig {
                enabled: true,
                ..archon_core::sandbox::DockerConfig::default()
            },
            archon_core::sandbox::DockerProbe::found("Docker 27.0.0"),
        );

        let body = render_sandbox_doctor(
            &[String::from("--backend"), String::from("docker")],
            SandboxDoctorOverrides {
                docker: Some(&override_report),
                ..SandboxDoctorOverrides::default()
            },
        );

        assert!(body.contains("Backend: docker"));
        assert!(body.contains("ready-detect-only"));
        assert!(body.contains("Execution: Bash routes through Docker"));
        assert!(body.contains("provider credentials"));
    }

    #[test]
    fn sandbox_doctor_ssh_reports_detect_only_status() {
        let override_report = (
            archon_core::sandbox::SshConfig {
                enabled: true,
                host: Some("sandbox.example".into()),
                user: Some("archon".into()),
                remote_workdir: Some("/srv/workspace".into()),
                ..archon_core::sandbox::SshConfig::default()
            },
            archon_core::sandbox::SshProbe::found("OpenSSH_9.6"),
        );

        let body = render_sandbox_doctor(
            &[String::from("--backend"), String::from("ssh")],
            SandboxDoctorOverrides {
                ssh: Some(&override_report),
                ..SandboxDoctorOverrides::default()
            },
        );

        assert!(body.contains("Backend: ssh"));
        assert!(body.contains("ready-detect-only"));
        assert!(body.contains("TOFU mismatch blocking"));
        assert!(body.contains("Execution: Bash routes through SSH"));
    }

    #[test]
    fn sandbox_doctor_openshell_reports_detect_only_status() {
        let override_report = (
            archon_core::sandbox::OpenShellConfig {
                enabled: true,
                ..archon_core::sandbox::OpenShellConfig::default()
            },
            archon_core::sandbox::OpenShellProbe::found("openshell 1.2.3"),
        );

        let body = render_sandbox_doctor(
            &[String::from("--backend"), String::from("openshell")],
            SandboxDoctorOverrides {
                openshell: Some(&override_report),
                ..SandboxDoctorOverrides::default()
            },
        );

        assert!(body.contains("Backend: openshell"));
        assert!(body.contains("ready-detect-only"));
        assert!(body.contains("Execution: disabled"));
        assert!(body.contains("Anthropic spoofing remains host-side"));
    }

    #[test]
    fn sandbox_doctor_logical_reports_policy_gate_only() {
        let body = render_sandbox_doctor(&[], SandboxDoctorOverrides::default());

        assert!(body.contains("Backend: logical"));
        assert!(body.contains("policy gate only"));
    }
}
