use super::*;

/// Carry authored statements independently of presentation headings. Named
/// sections remain a compatibility adapter for unkeyed legacy prose.
pub(super) fn source_rules(body: &str) -> Vec<String> {
    let prepared = crate::obligation_ids::prepare_markdown(body);
    let mut rules: Vec<_> = crate::obligation_ids::obligation_texts(body)
        .into_values()
        .collect();
    rules.extend(crate::obligation_ids::constraint_texts(body).into_values());
    rules.extend(extract_sections(&prepared, &["Hard Rules", "Constraints"]));
    rules.extend(prepared.lines().filter_map(|line| {
        let line = line.trim();
        line.strip_prefix("- ")
            .or_else(|| line.strip_prefix("* "))
            .or_else(|| line.strip_prefix("+ "))
            .map(str::to_string)
    }));
    rules.sort();
    rules.dedup();
    rules
}
