//! PRD obligation identifiers shared by decomposition, lint, and trace.
//!
//! Identifiers are taken from declarations in the source, never a prefix list.
//! Tables and list definitions share the same token reader. Coverage resolves
//! exact membership in this inventory rather than reimplementing its grammar.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::OnceLock;

#[path = "obligation_source_sections.rs"]
mod source_sections;
pub use source_sections::obligation_source_sections;

use regex::Regex;

#[path = "obligation_continuations.rs"]
mod continuations;

#[path = "prd_markdown.rs"]
mod markdown;
pub(crate) fn prepare_markdown(prd: &str) -> String {
    markdown::prepare(prd, true)
}

#[path = "obligation_constraints.rs"]
mod constraints;
pub use constraints::{constraint_ids, constraint_texts};

#[path = "prd_sections.rs"]
mod prd_sections;
use constraints::HeadingExclusion;
pub use prd_sections::{PrdSection, numbered_sections, section_citation_findings};

const EXCLUDED_HEADINGS: [&str; 5] = [
    "non-goal",
    "out of scope",
    "excluded",
    "deviation",
    "anti-goal",
];
/// Prefix of the synthetic id minted for each numbered done-definition item.
pub const DONE_ITEM_PREFIX: &str = "DONE-";

pub fn obligation_ids(prd: &str) -> BTreeSet<String> {
    let mut ids = bullet_requirement_ids(prd);
    ids.extend(table_obligation_ids(prd));
    ids.extend(done_items(prd).into_iter().map(|(id, _)| id));
    ids
}

/// Every obligation the PRD states, keyed by id, with its exact text.
///
/// The text is what a semantic audit reads: the bullet after its id, the
/// criterion or goal cell beside a table id, the done item after its number.
/// The first statement of an id wins, matching [`acceptance_criteria`].
pub fn obligation_texts(prd: &str) -> BTreeMap<String, String> {
    let mut texts = BTreeMap::new();
    for (id, text) in bullet_requirement_entries(prd)
        .into_iter()
        .chain(table_obligation_rows(prd))
        .chain(done_items(prd))
    {
        texts.entry(id).or_insert(text);
    }
    texts
}

pub fn acceptance_ids(prd: &str) -> BTreeSet<String> {
    acceptance_criteria(prd).into_keys().collect()
}

/// Exact acceptance criterion text keyed by its PRD table ID.
pub fn acceptance_criteria(prd: &str) -> BTreeMap<String, String> {
    let prepared = prepare_markdown(prd);
    let mut result = BTreeMap::new();
    let mut acceptance_table = false;
    let mut exclusion = HeadingExclusion::default();
    for line in prepared.lines() {
        let trimmed = line.trim_start();
        if trimmed.starts_with('#') {
            exclusion.update(trimmed);
        }
        if !trimmed.starts_with('|') {
            acceptance_table = false;
            continue;
        }
        if let Some(id) = table_candidate_id(trimmed) {
            if acceptance_table && !exclusion.active() {
                if let Some(text) = trimmed.trim_matches('|').split('|').nth(1) {
                    result
                        .entry(id.to_string())
                        .or_insert_with(|| text.trim().to_string());
                }
            }
        } else if !trimmed.contains("---") {
            let header = trimmed.to_lowercase();
            acceptance_table = header.contains("criterion")
                || header.contains("criteria")
                || header.contains("acceptance");
        }
    }
    result
}

/// Read an authored ID, including inline emphasis and code delimiters. The
/// prefix, separators and numeric width are retained exactly as declared.
fn authored_id(value: &str) -> Option<&str> {
    let id = value.trim().trim_matches(['*', '_', '`']);
    let first = id.chars().next()?;
    (first.is_alphabetic()
        && id.chars().last()?.is_alphanumeric()
        && (id.chars().any(char::is_numeric) || id.contains(['-', '_', '.', ':', '/', '~', '+']))
        && id
            .chars()
            .all(|ch| ch.is_alphanumeric() || "-_.:/~+".contains(ch)))
    .then_some(id)
}

fn table_candidate_id(line: &str) -> Option<&str> {
    authored_id(line.trim().strip_prefix('|')?.split('|').next()?)
}

pub fn duplicate_obligation_finding(id: &str) -> String {
    format!(
        "source obligation id '{id}' appears in more than one obligation-table row; retain the authored id while the harness resolves the conflicting statements before claiming coverage"
    )
}

pub fn duplicate_obligation_ids(prd: &str) -> Vec<String> {
    let mut counts = BTreeMap::<String, usize>::new();
    for id in table_obligation_row_ids(prd) {
        *counts.entry(id).or_default() += 1;
    }
    counts
        .into_iter()
        .filter_map(|(id, count)| (count > 1).then_some(id))
        .collect()
}

pub fn malformed_obligation_finding(id: &str) -> String {
    format!(
        "source identifier '{id}' could not be resolved against the document's declarations; the harness must retain the authored identifier and adapt its inventory"
    )
}

/// There is no external canonical prefix or width to impose on authored IDs.
pub fn malformed_obligation_ids(_prd: &str) -> Vec<String> {
    Vec::new()
}

fn bullet_requirement_ids(prd: &str) -> BTreeSet<String> {
    bullet_requirement_entries(prd)
        .into_iter()
        .map(|(id, _)| id)
        .collect()
}

fn bullet_requirement_entries(prd: &str) -> Vec<(String, String)> {
    let prepared = prepare_markdown(prd);
    let lines: Vec<_> = prepared.lines().collect();
    let mut exclusion = HeadingExclusion::default();
    let mut entries = Vec::new();
    for (index, line) in lines.iter().enumerate() {
        let trimmed = line.trim_start();
        if trimmed.starts_with('#') {
            exclusion.update(trimmed);
            continue;
        }
        if exclusion.active() || !is_list_item(trimmed) {
            continue;
        }
        let body = continuations::list_item_body(trimmed);
        let Some((id, first)) = constraints::definition(body) else {
            continue;
        };
        let mut text = vec![first.to_string()];
        continuations::append(&lines, index + 1, line.len() - trimmed.len(), &mut text);
        entries.push((id.to_string(), continuations::join(&text)));
    }
    entries
}

/// Top-level numbered source statements retain positional `DONE-<n>` IDs.
/// Numbered definitions with their own ID use that ID instead. Section titles
/// do not decide whether a structured statement requires ownership.
fn done_items(prd: &str) -> Vec<(String, String)> {
    done_statements(prd)
        .into_iter()
        .map(|(id, text, _)| (id, text))
        .collect()
}

fn done_statements(prd: &str) -> Vec<(String, String, usize)> {
    let mut items = Vec::new();
    let mut exclusion = HeadingExclusion::default();
    let prepared = prepare_markdown(prd);
    let lines: Vec<_> = prepared.lines().collect();
    for (index, line) in lines.iter().enumerate() {
        let trimmed = line.trim_start();
        if trimmed.starts_with('#') {
            exclusion.update(trimmed);
            continue;
        }
        if exclusion.active()
            || line.len() - trimmed.len() >= 2
            || constraints::leading_id(continuations::list_item_body(trimmed)).is_some()
        {
            continue;
        }
        if let Some(text) = numbered_item_text(trimmed) {
            let indentation = line.len() - trimmed.len();
            let mut wrapped = vec![text];
            continuations::append(&lines, index + 1, indentation, &mut wrapped);
            items.push((
                format!("{DONE_ITEM_PREFIX}{}", items.len() + 1),
                continuations::join(&wrapped),
                index + 1,
            ));
        }
    }
    items
}

fn is_setext_underline(line: &str) -> bool {
    let line = line.trim();
    !line.is_empty() && (line.chars().all(|ch| ch == '=') || line.chars().all(|ch| ch == '-'))
}

fn is_list_item(line: &str) -> bool {
    let Some(first) = line.chars().next() else {
        return false;
    };
    if matches!(first, '-' | '*' | '+') {
        return line.chars().nth(1).is_some_and(char::is_whitespace);
    }
    let digits = line.chars().take_while(char::is_ascii_digit).count();
    digits > 0
        && line[digits..]
            .chars()
            .next()
            .is_some_and(|ch| matches!(ch, '.' | ')'))
        && line[digits + 1..]
            .chars()
            .next()
            .is_some_and(char::is_whitespace)
}

fn numbered_item_text(line: &str) -> Option<String> {
    let digits = line.chars().take_while(char::is_ascii_digit).count();
    if digits == 0 {
        return None;
    }
    let rest = line[digits..].strip_prefix(['.', ')'])?;
    let text = rest.trim();
    (!text.is_empty()).then(|| text.to_string())
}

fn table_obligation_ids(prd: &str) -> BTreeSet<String> {
    table_obligation_entries(prd).into_keys().collect()
}

fn table_obligation_entries(prd: &str) -> BTreeMap<String, String> {
    let mut entries = BTreeMap::new();
    for (id, criterion) in table_obligation_rows(prd) {
        entries.entry(id).or_insert(criterion);
    }
    entries
}

fn table_obligation_row_ids(prd: &str) -> Vec<String> {
    let mut ids = Vec::new();
    let mut exclusion = HeadingExclusion::default();
    let mut states_obligations = None;
    let prepared = prepare_markdown(prd);
    for line in prepared.lines() {
        let trimmed = line.trim_start();
        if trimmed.starts_with('#') {
            exclusion.update(trimmed);
            states_obligations = None;
            continue;
        }
        if !trimmed.starts_with('|') {
            states_obligations = None;
            continue;
        }
        let states = *states_obligations.get_or_insert_with(|| header_states_obligations(line));
        if exclusion.active() || !states {
            continue;
        }
        let cells = line
            .trim()
            .trim_matches('|')
            .split('|')
            .map(str::trim)
            .collect::<Vec<_>>();
        if let Some(id) = cells.first().copied().and_then(authored_id) {
            ids.push(id.to_string());
        }
    }
    ids
}

fn table_obligation_rows(prd: &str) -> Vec<(String, String)> {
    let mut rows = Vec::new();
    let mut exclusion = HeadingExclusion::default();
    let mut states_obligations = None;
    let prepared = prepare_markdown(prd);
    for line in prepared.lines() {
        let trimmed = line.trim_start();
        if trimmed.starts_with('#') {
            exclusion.update(trimmed);
            states_obligations = None;
            continue;
        }
        if !trimmed.starts_with('|') {
            states_obligations = None;
            continue;
        }
        let states = *states_obligations.get_or_insert_with(|| header_states_obligations(line));
        if exclusion.active() || !states {
            continue;
        }
        let cells: Vec<_> = line
            .trim()
            .trim_matches('|')
            .split('|')
            .map(str::trim)
            .collect();
        let Some(id) = cells.first().copied().and_then(authored_id) else {
            continue;
        };
        if let Some(criterion) = cells.get(1).filter(|value| !value.is_empty()) {
            rows.push((id.to_string(), (*criterion).to_string()));
        }
    }
    rows
}

fn heading_excludes_obligations(line: &str) -> bool {
    let lower = line.trim_start_matches('#').trim().to_ascii_lowercase();
    EXCLUDED_HEADINGS.iter().any(|word| lower.contains(word))
}

/// A header naming an obligation, unless it names an exclusion: "non-goal"
/// contains "goal", and a non-goals table under a neutral heading must not
/// become a set of goals the moment goal tables count.
fn header_states_obligations(line: &str) -> bool {
    let lower = line.to_lowercase();
    !heading_excludes_obligations(line)
        && [
            "criterion",
            "criteria",
            "requirement",
            "obligation",
            "acceptance",
            "must",
            "goal",
        ]
        .iter()
        .any(|word| lower.contains(word))
}

/// Forbidden residual-gap phrases declared by the PRD itself.
///
/// The list begins at a heading containing "residual" and "gap", after a line
/// naming forbidden phrases, and ends at the next heading. The workflow engine
/// carries no phrase vocabulary of its own.
pub fn residual_gap_forbidden_phrases(prd: &str) -> Vec<String> {
    let mut in_list = false;
    let mut phrases = Vec::new();
    let prepared = prepare_markdown(prd);
    for line in prepared.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('#') {
            in_list = false;
            continue;
        }
        let lower = trimmed.to_ascii_lowercase();
        if lower.contains("forbidden") && lower.contains("phrase") {
            in_list = true;
            continue;
        }
        if !in_list {
            continue;
        }
        let Some(value) = trimmed
            .strip_prefix("- ")
            .or_else(|| trimmed.strip_prefix("* "))
        else {
            continue;
        };
        let value = value.trim();
        let value = quoted_phrase(value).unwrap_or_else(|| {
            value
                .split_once(" without ")
                .map(|(phrase, _)| phrase)
                .unwrap_or(value)
                .trim()
                .trim_matches(['\'', '"', '`'])
        });
        if !value.is_empty() && !phrases.iter().any(|existing| existing == value) {
            phrases.push(value.to_string());
        }
    }
    phrases
}

fn quoted_phrase(value: &str) -> Option<&str> {
    let quote = value.chars().next()?;
    if !matches!(quote, '\'' | '"' | '`') {
        return None;
    }
    let rest = &value[quote.len_utf8()..];
    let end = rest.find(quote)?;
    Some(rest[..end].trim())
}

#[cfg(test)]
#[path = "obligation_ids_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "obligation_format_tests.rs"]
mod format_tests;

/// One-based declaration lines, using the same ID reader and code boundaries.
pub fn obligation_source_lines(prd: &str) -> BTreeMap<String, usize> {
    let mut known = obligation_ids(prd);
    known.extend(constraint_ids(prd));
    let prepared = prepare_markdown(prd);
    let mut result = BTreeMap::new();
    let mut exclusion = HeadingExclusion::default();
    let mut table_excluded = None;
    for (index, line) in prepared.lines().enumerate() {
        let trimmed = line.trim_start();
        if trimmed.starts_with('#') {
            exclusion.update(trimmed);
        }
        if !trimmed.starts_with('|') {
            table_excluded = None;
        }
        if exclusion.active() {
            continue;
        }
        if trimmed.starts_with('|')
            && *table_excluded.get_or_insert_with(|| heading_excludes_obligations(trimmed))
        {
            continue;
        }
        let id = if is_list_item(trimmed) {
            constraints::leading_id(continuations::list_item_body(trimmed))
        } else {
            table_candidate_id(trimmed)
        };
        if let Some(id) = id.filter(|id| known.contains(*id)) {
            result.entry(id.to_string()).or_insert(index + 1);
        }
    }
    for (id, _, line) in done_statements(prd) {
        result.entry(id).or_insert(line);
    }
    result
}
