use super::*;

impl VerifyCommandReport {
    pub(crate) fn success(&self) -> bool {
        matches!(
            crate::acceptance_check_crash::classify_execution(
                &self.command,
                self.exit_code,
                &self.stderr,
                &[]
            ),
            crate::acceptance_check_crash::CheckRunClass::Passed
        ) && self.parse_error.is_none()
            && !command_is_discovery_only(&self.command)
            && !output_reports_zero_work(&self.stdout, &self.stderr)
    }

    pub(crate) fn failure_reason(&self) -> String {
        // Issue 554: a command its shell cannot parse never asserted anything.
        if let Some(error) = &self.parse_error {
            return format!(
                "verify_command does not parse in the shell that runs it (`-n`), so it was not run. The rule: {}. The shell reported: {error}",
                crate::acceptance_check_shell::PARSE_RULE
            );
        }
        if self.exit_code == Some(0) && command_is_discovery_only(&self.command) {
            return "verify_command is discovery/list-only and cannot prove completion".into();
        }
        if self.exit_code == Some(0) && output_reports_zero_work(&self.stdout, &self.stderr) {
            return "verify_command produced zero-test/no-op output and cannot prove completion"
                .into();
        }
        if self.exit_code == Some(0) && !self.success() {
            return "verify_command failed during shell expansion despite status 0".into();
        }
        let code = self
            .exit_code
            .map(|code| code.to_string())
            .unwrap_or_else(|| "signal".to_string());
        format!(
            "verify_command exited with status {code}{}",
            self.environment_note
                .as_ref()
                .map(|note| format!("\n{note}"))
                .unwrap_or_default()
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn masked_abort_is_not_successful_verification() {
        let report = VerifyCommandReport {
            command: "set -u; trap 'exit 0' EXIT; : \"${X:?msg}\"".into(),
            exit_code: Some(0),
            stdout: String::new(),
            stderr: "/bin/sh: X: msg".into(),
            environment_note: None,
            parse_error: None,
        };
        assert!(!report.success());
        assert!(report.failure_reason().contains("shell expansion"));
    }
}
