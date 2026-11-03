//! PRD list-item and table-row constraints share the obligation identifier shape.

use super::*;

/// Shared depth-aware exclusion for constraint and section extraction.
#[derive(Default)]
pub(super) struct HeadingExclusion(Option<usize>);
impl HeadingExclusion {
    pub(super) fn update(&mut self, heading: &str) {
        let level = heading.chars().take_while(|ch| *ch == '#').count();
        if self.0.is_none_or(|open| level <= open) {
            self.0 = heading_excludes_obligations(heading).then_some(level);
        }
    }
    pub(super) fn active(&self) -> bool {
        self.0.is_some()
    }
}

/// List-item and table-row ids other than obligations, outside excluded sections.
pub fn constraint_ids(prd: &str) -> BTreeSet<String> {
    constraint_texts(prd).into_keys().collect()
}

/// Exact constraint text, joined using the obligation continuation rules.
/// The first statement wins. Exclusions include all deeper subsections.
pub fn constraint_texts(prd: &str) -> BTreeMap<String, String> {
    let obligations = obligation_ids(prd);
    let lines: Vec<_> = prd.lines().collect();
    let mut exclusion = HeadingExclusion::default();
    let mut excluded_table = None;
    let mut texts = BTreeMap::new();
    for (index, line) in lines.iter().enumerate() {
        let trimmed = line.trim_start();
        if trimmed.starts_with('#') {
            exclusion.update(trimmed);
            excluded_table = None;
            continue;
        }
        if !trimmed.starts_with('|') {
            excluded_table = None;
        } else {
            // A non-goal header under a neutral heading excludes the table too.
            let excluded_table =
                *excluded_table.get_or_insert_with(|| heading_excludes_obligations(trimmed));
            if !exclusion.active()
                && !excluded_table
                && let Some(id) = table_candidate_id(trimmed)
                && generic_table_obligation_pattern().is_match(id)
                && !is_req_or_ac_id(id)
                && !obligations.contains(id)
            {
                let text = trimmed
                    .trim()
                    .trim_matches('|')
                    .split('|')
                    .skip(1)
                    .map(str::trim)
                    .collect::<Vec<_>>()
                    .join(" | ");
                texts.entry(id.to_string()).or_insert(text);
            }
            continue;
        }
        if exclusion.active() || !is_list_item(trimmed) {
            continue;
        }
        let body = continuations::list_item_body(trimmed);
        let Some(id) = leading_id(body) else {
            continue;
        };
        if is_req_or_ac_id(id) || obligations.contains(id) {
            continue;
        }
        let first = body[id.len()..]
            .trim_start()
            .trim_start_matches([':', '—', '–', '-'])
            .trim();
        let mut text = vec![first.to_string()];
        continuations::append(&lines, index + 1, line.len() - trimmed.len(), &mut text);
        texts
            .entry(id.to_string())
            .or_insert_with(|| continuations::join(&text));
    }
    texts
}

/// A definition-form leading token using the shared generic shape.
/// Bold ids are not read; fenced code is not skipped, as with obligations.
pub(super) fn leading_id(body: &str) -> Option<&str> {
    let end = body
        .find(|ch: char| !(ch.is_ascii_alphanumeric() || ch == '-'))
        .unwrap_or(body.len());
    let id = &body[..end];
    let definition = body[end..].trim_start().strip_prefix([':', '—', '–'])?;
    (generic_table_obligation_pattern().is_match(id) && !definition.trim().is_empty()).then_some(id)
}

#[cfg(test)]
#[path = "obligation_constraints_tests.rs"]
mod tests;
