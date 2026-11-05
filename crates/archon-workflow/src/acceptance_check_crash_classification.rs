use super::*;

/// Classify one executed check. `command` is the exact check text the host
/// ran (a command check's command or a floor's verifier command).
pub fn classify_check_run(command: &str, result: &CheckResult) -> CheckRunClass {
    classify_with_environment(command, result, &[])
}

fn classify_with_environment(command: &str, result: &CheckResult, bound: &[&str]) -> CheckRunClass {
    if result.operational_error.is_some() {
        return CheckRunClass::Failed;
    }
    if let Some(classification) = &result.classification {
        return classification.crash.clone();
    }
    classify_execution(
        command,
        result.exit_code,
        &String::from_utf8_lossy(result.verdict_stderr()),
        bound,
    )
}

/// Classify an execution from its raw stderr before any evidence redaction.
pub fn classify_execution(
    command: &str,
    exit_code: Option<i32>,
    stderr: &str,
    bound: &[&str],
) -> CheckRunClass {
    let Some(code) = exit_code else {
        return CheckRunClass::Failed;
    };
    let lines: Vec<&str> = stderr.lines().filter(|l| !l.trim().is_empty()).collect();
    if let Some(signal) = lines
        .iter()
        .find(|line| crate::acceptance_check_shell_unset::runtime_unset(command, code, line, bound))
    {
        return CheckRunClass::ScriptDefect(ScriptDefect {
            interpreter: "shell".into(),
            rule: "fatal expansion in the check shell".into(),
            signal: signal.trim().to_string(),
        });
    }
    if code == 0 {
        return if crate::acceptance_check_shell_unset::never_pass(command, code, stderr) {
            CheckRunClass::Failed
        } else {
            CheckRunClass::Passed
        };
    }
    python_defect(command, code, &lines)
        .or_else(|| shell_defect(command, code, &lines))
        .map_or(CheckRunClass::Failed, CheckRunClass::ScriptDefect)
}

/// Host-computed verdict from raw streams, preserved across redaction/reuse.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CheckClassification {
    pub passed: bool,
    pub zero_work: bool,
    pub crash: CheckRunClass,
    /// Issue 544: the failure's regression fingerprint, computed from the
    /// raw capture with each secret as an indexed placeholder, so stored
    /// results compare as the raw ones would without holding a secret.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failure_signature: Option<String>,
}

impl CheckResult {
    /// The capture's verdict, shared by observation, probes and live rounds.
    pub fn passed(&self) -> bool {
        self.operational_error.is_none()
            && self.classification.as_ref().map_or_else(
                || {
                    matches!(classify_check_run("", self), CheckRunClass::Passed)
                        && !self.reports_zero_work()
                },
                |classification| classification.passed,
            )
    }

    /// Call before changing any captured stream. The command is host-authorized.
    pub fn classify_raw(&mut self, command: &str) {
        self.classify_raw_with_environment(command, &[]);
    }

    pub(crate) fn classify_raw_with_environment(&mut self, command: &str, bound: &[&str]) {
        if self.classification.is_some() {
            return;
        }
        let zero_work = crate::acceptance::output_reports_zero_work(
            &String::from_utf8_lossy(self.verdict_stdout()),
            &String::from_utf8_lossy(self.verdict_stderr()),
        );
        let mut crash = classify_with_environment(command, self, bound);
        if let CheckRunClass::ScriptDefect(defect) = &mut crash {
            // Raw diagnostic text belongs only in the redacted evidence.
            defect.signal = "see the fenced stderr below".into();
        }
        self.classification = Some(CheckClassification {
            passed: matches!(crash, CheckRunClass::Passed) && !zero_work,
            zero_work,
            crash,
            failure_signature: None,
        });
    }
}
