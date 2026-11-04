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
    let forms = super::super::executability::test_inventory::cargo_multi_names(command);
    let reason = if program == "cargo"
        && rejected.contains("unexpected argument")
        && !forms.is_empty()
    {
        rule.push_str(&format!("; rejected invocations: {}", forms.join("; ")));
        "`cargo test` takes one TESTNAME before `--`; put names after `-- --exact` or run one `cargo test` per name".to_string()
    } else {
        format!(
            "nothing the repository implements can change how {program} parses its arguments; fix the invocation; every other invocation with the same form is rejected the same way"
        )
    };
    (rule, reason)
}
