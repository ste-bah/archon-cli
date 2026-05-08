pub(crate) type OpenShellDoctorOverride = (
    archon_core::sandbox::OpenShellConfig,
    archon_core::sandbox::OpenShellProbe,
);

pub(crate) fn render_sandbox_doctor(
    args: &[String],
    openshell_override: Option<&OpenShellDoctorOverride>,
) -> String {
    let backend = parse_backend_arg(args).unwrap_or("logical");
    match backend {
        "openshell" => render_openshell_doctor(openshell_override),
        "logical" => {
            "Sandbox doctor\nBackend: logical\nStatus: available\nIsolation: policy gate only, not OS/container isolation\nExecution: host tool execution remains governed by permission preflight\n".into()
        }
        other => {
            format!("Sandbox doctor\nBackend: {other}\nStatus: not implemented in this build\n")
        }
    }
}

fn render_openshell_doctor(openshell_override: Option<&OpenShellDoctorOverride>) -> String {
    if let Some((config, probe)) = openshell_override {
        let report = archon_core::sandbox::openshell_doctor_report(config, probe.clone());
        return archon_core::sandbox::render_openshell_doctor_report(&report);
    }

    let config = match load_openshell_config_without_writing() {
        Ok(config) => config,
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

fn load_openshell_config_without_writing() -> Result<archon_core::sandbox::OpenShellConfig, String>
{
    let path = archon_core::config::default_config_path();
    if !path.exists() {
        return Ok(archon_core::sandbox::OpenShellConfig::default());
    }
    archon_core::config::load_config_if_exists(path)
        .map(|config| {
            config
                .map(|config| config.sandbox.openshell)
                .unwrap_or_default()
        })
        .map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

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
            Some(&override_report),
        );

        assert!(body.contains("Backend: openshell"));
        assert!(body.contains("ready-detect-only"));
        assert!(body.contains("Execution: disabled"));
        assert!(body.contains("Anthropic spoofing remains host-side"));
    }

    #[test]
    fn sandbox_doctor_logical_reports_policy_gate_only() {
        let body = render_sandbox_doctor(&[], None);

        assert!(body.contains("Backend: logical"));
        assert!(body.contains("policy gate only"));
    }
}
