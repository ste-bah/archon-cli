use super::*;

/// Only explicitly declared hard-rule and constraint sections feed intake.
pub(super) fn source_rules(body: &str) -> Vec<String> {
    extract_sections(body, &["Hard Rules", "Constraints"])
}
