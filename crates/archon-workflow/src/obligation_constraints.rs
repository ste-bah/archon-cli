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
    let prepared = prepare_markdown(prd);
    let mut texts = BTreeMap::new();
    let mut exclusion = HeadingExclusion::default();
    let mut constraint_table = false;
    for line in prepared.lines() {
        let trimmed = line.trim_start();
        if trimmed.starts_with('#') {
            exclusion.update(trimmed);
        }
        if !trimmed.starts_with('|') {
            constraint_table = false;
            continue;
        }
        if let Some(id) = table_candidate_id(trimmed) {
            if constraint_table && !exclusion.active() {
                let text = trimmed
                    .trim_matches('|')
                    .split('|')
                    .skip(1)
                    .map(str::trim)
                    .collect::<Vec<_>>()
                    .join(" | ");
                texts.entry(id.to_string()).or_insert(text);
            }
        } else if !trimmed.contains("---") {
            constraint_table =
                !header_states_obligations(trimmed) && !heading_excludes_obligations(trimmed);
        }
    }
    texts
}

/// Definition boundaries distinguish declarations from references in prose.
pub(super) fn definition(body: &str) -> Option<(&str, &str)> {
    let end = body.find(char::is_whitespace).unwrap_or(body.len());
    let raw = &body[..end];
    let unwrapped = raw.trim_matches(['*', '_', '`']);
    let token = unwrapped.trim_end_matches([':', '—', '–']);
    let token = token.trim_matches(['*', '_', '`']);
    let id = authored_id(token)?;
    let rest = body[end..].trim_start();
    let attached = unwrapped.trim_end_matches(['*', '_', '`']) != token;
    let text = if attached {
        rest
    } else {
        rest.strip_prefix([':', '—', '–', '-'])?.trim_start()
    };
    (!text.is_empty()).then_some((id, text))
}

pub(super) fn leading_id(body: &str) -> Option<&str> {
    definition(body).map(|(id, _)| id)
}

#[cfg(test)]
#[path = "obligation_constraints_tests.rs"]
mod tests;
