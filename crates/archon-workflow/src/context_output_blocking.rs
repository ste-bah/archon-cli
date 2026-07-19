const AUDIT_IMPOSSIBLE_PHRASES: &[&str] = &[
    "cannot audit",
    "could not audit",
    "can't audit",
    "unable to audit",
    "missing required evidence",
    "missing source evidence",
    "source evidence is missing",
    "source files are missing",
    "source files or upstream artifacts are absent",
    "no source evidence",
    "no source files",
    "no file content",
    "insufficient context",
    "no tool execution results are available",
    "cannot truthfully report pass/fail",
    "without executing the commands",
];

const CONFIRMATION_PHRASES: &[&str] = &[
    "would you like me to proceed",
    "do you want me to proceed",
    "should i proceed",
    "shall i proceed",
    "would you like me to continue",
    "do you want me to continue",
    "let me know if you want me to proceed",
    "let me know if you'd like me to proceed",
    "if you want me to proceed",
];

pub(super) fn reports_missing_evidence_block(lower: &str) -> bool {
    (lower.contains("findings: []") || lower.contains("\"findings\":[]"))
        && text_has_any(lower, AUDIT_IMPOSSIBLE_PHRASES)
}

pub(super) fn reports_waiting_for_confirmation(lower: &str) -> bool {
    text_has_any(lower, CONFIRMATION_PHRASES)
}

fn text_has_any(text: &str, needles: &[&str]) -> bool {
    needles.iter().any(|needle| text.contains(needle))
}
