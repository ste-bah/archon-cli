use super::*;

pub(crate) fn parser_diagnosis(
    program: &str,
    path: &Path,
    rejected: &str,
    command: &str,
) -> (String, String) {
    let mut rule = format!(
        "{program} ({}, outside the repository) rejected the check's command line: {rejected}",
        archon_shell::paths::stored_text(path)
    );
    let diagnosis = command_diagnosis(command);
    let reason = if let Some((invocations, reason)) =
        diagnosis.filter(|_| program == "cargo" && rejected.contains("unexpected argument"))
    {
        rule.push_str(&format!("; {invocations}"));
        reason
    } else {
        format!(
            "nothing the repository implements can change how {program} parses its arguments; fix the invocation; every other invocation with the same form is rejected the same way"
        )
    };
    (rule, reason)
}

/// Only the multiple positional-name rule is decidable without run evidence.
/// General parser refusals still require output and external-program identity.
fn command_diagnosis(command: &str) -> Option<(String, String)> {
    diagnosis(super::super::executability::test_inventory::cargo_multi_names(command))
}

fn diagnosis(forms: Vec<String>) -> Option<(String, String)> {
    (!forms.is_empty()).then(|| (
        format!("invocations cargo rejects: {}", forms.join("; ")),
        "`cargo test` takes one TESTNAME before `--`; put names after `-- --exact` or run one `cargo test` per name".to_string(),
    ))
}

const ARGV_LINT_CALL: &str = "host:argument-text-usage";

pub(crate) fn lint_finding(entry: &AcceptanceCriterion) -> Option<&str> {
    (entry.judgment.verdict == JudgeDecision::Refuted
        && entry.judgment.host_call_id == ARGV_LINT_CALL)
        .then_some(entry.judgment.reason.as_str())
}

/// Overwrite even accepted verdicts before consulting any judgment cache.
pub(crate) fn lint_commands(contract: &mut AcceptanceContract) -> BTreeSet<String> {
    let mut linted = BTreeSet::new();
    for entry in contract
        .acceptance
        .iter_mut()
        .chain(&mut contract.supplementary)
    {
        // Passability's check_command only covers Command checks, not Floor verifiers.
        if !matches!(
            &entry.check,
            archon_workflow::task_set_contract::AcceptanceCheck::Command { .. }
        ) {
            continue;
        }
        let Some((_, command)) = super::super::executability::executed_text(entry) else {
            continue;
        };
        let forms = super::super::executability::test_inventory::cargo_lint_forms(command);
        let Some((rule, reason)) = diagnosis(forms) else {
            continue;
        };
        entry.judgment.verdict = JudgeDecision::Refuted;
        entry.judgment.counterexample = rule.clone();
        entry.judgment.reason = format!(
            "check '{}': {CANNOT_PASS}: rule: \"{rule}\"; reason: \"{reason}\"",
            entry.id
        );
        entry.judgment.host_call_id = ARGV_LINT_CALL.to_string();
        entry.judgment.sampling = None;
        linted.insert(entry.id.clone());
    }
    linted
}
