//! PRD obligation identifiers shared by decomposition, lint, and trace.
//!
//! Three shapes of obligation are read, and every consumer sees the same union:
//!
//! - line-leading `REQ-` bullets;
//! - rows of any table whose header names an obligation — a criterion, a
//!   requirement, an acceptance, a goal;
//! - the numbered items under a "done" heading, as synthetic `DONE-<n>` ids.
//!
//! # Why goals and done-items are obligations
//!
//! Observed live: a PRD stated its goals in a `| ID | Goal |` table and its
//! completion contract as fourteen numbered "Done Definition" items. Neither
//! was extracted, so no task claimed them, no gate asked who owned them, and
//! every task passed its own acceptance while the registry the goal named
//! held zero entries. A goal nobody claims is work nobody does, exactly like a
//! requirement nobody claims; the extractor treats them alike so coverage can.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::OnceLock;

use regex::Regex;

const EXCLUDED_HEADINGS: [&str; 5] = [
    "non-goal",
    "out of scope",
    "excluded",
    "deviation",
    "anti-goal",
];
const OBLIGATION_HEADER_WORDS: [&str; 7] = [
    "criterion",
    "criteria",
    "requirement",
    "obligation",
    "acceptance",
    "must",
    "goal",
];
/// A heading containing "done" and one of these names the completion contract.
const DONE_HEADING_QUALIFIERS: [&str; 3] = ["definition", "when", "criteria"];
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
    table_obligation_entries(prd)
        .into_iter()
        .filter(|(id, _)| id.starts_with("AC-"))
        .collect()
}

fn exact_req_or_ac_pattern() -> &'static Regex {
    static PATTERN: OnceLock<Regex> = OnceLock::new();
    PATTERN.get_or_init(|| {
        Regex::new(r"^(?:REQ|AC)-[A-Z]+-[0-9]{3}$").expect("exact REQ/AC regex is literal")
    })
}

/// One or more family letters: a goals table conventionally keys its rows
/// `G-<AREA>-<NNN>`, and requiring two letters made every goal id malformed
/// the moment goal tables began to count.
fn generic_table_obligation_pattern() -> &'static Regex {
    static PATTERN: OnceLock<Regex> = OnceLock::new();
    PATTERN.get_or_init(|| {
        Regex::new(r"^[A-Z][A-Z0-9]*(?:-[A-Z0-9]+)?-[0-9]{3}$")
            .expect("generic table obligation regex is literal")
    })
}

fn valid_table_obligation_id(id: &str) -> bool {
    if id.starts_with("REQ-") || id.starts_with("AC-") {
        exact_req_or_ac_pattern().is_match(id)
    } else {
        generic_table_obligation_pattern().is_match(id)
    }
}

fn requirement_candidate_pattern() -> &'static Regex {
    static PATTERN: OnceLock<Regex> = OnceLock::new();
    PATTERN.get_or_init(|| {
        Regex::new(r"(?m)^[ \\t]*[-*][ \\t]+(REQ-[A-Za-z0-9-]+)")
            .expect("requirement candidate regex is literal")
    })
}

fn table_candidate_id(line: &str) -> Option<&str> {
    line.trim()
        .strip_prefix('|')?
        .split('|')
        .next()
        .map(str::trim)
        .filter(|id| {
            id.starts_with(|ch: char| ch.is_ascii_uppercase())
                && id.chars().all(|ch| ch.is_ascii_alphanumeric() || ch == '-')
                && id.chars().any(|ch| ch.is_ascii_digit())
        })
}

pub fn duplicate_obligation_finding(id: &str) -> String {
    format!(
        "PRD obligation id '{id}' appears in more than one obligation-table row; keep exactly one row for that id or assign distinct canonical IDs to independently testable criteria"
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
        "PRD obligation id '{id}' is malformed; rename it to exact REQ-<LETTERS>-<NNN> for a requirement bullet or <FAMILY>-<LETTERS>-<NNN> for an obligation-table row"
    )
}

pub fn malformed_obligation_ids(prd: &str) -> Vec<String> {
    let mut malformed = BTreeSet::new();
    for capture in requirement_candidate_pattern().captures_iter(prd) {
        let id = capture[1].to_string();
        if !exact_req_or_ac_pattern().is_match(&id) {
            malformed.insert(id);
        }
    }

    let mut excluded = false;
    let mut states_obligations = None;
    for line in prd.lines() {
        let trimmed = line.trim_start();
        if trimmed.starts_with('#') {
            excluded = heading_excludes_obligations(trimmed);
            states_obligations = None;
            continue;
        }
        if !trimmed.starts_with('|') {
            states_obligations = None;
            continue;
        }
        let states = *states_obligations.get_or_insert_with(|| header_states_obligations(line));
        if excluded || !states {
            continue;
        }
        if let Some(id) = table_candidate_id(line)
            && !valid_table_obligation_id(id)
        {
            malformed.insert(id.to_string());
        }
    }
    malformed.into_iter().collect()
}

fn bullet_requirement_ids(prd: &str) -> BTreeSet<String> {
    requirement_candidate_pattern()
        .captures_iter(prd)
        .map(|capture| capture[1].to_string())
        .filter(|id| exact_req_or_ac_pattern().is_match(id))
        .collect()
}

/// Each well-formed `REQ-` bullet with the text that follows its id on the
/// same line, separator (`:`, `—`, `-`) stripped.
fn bullet_requirement_entries(prd: &str) -> Vec<(String, String)> {
    requirement_candidate_pattern()
        .captures_iter(prd)
        .filter_map(|capture| {
            let id = capture.get(1)?;
            if !exact_req_or_ac_pattern().is_match(id.as_str()) {
                return None;
            }
            let rest = &prd[id.end()..];
            let line = rest.split(['\n', '\r']).next().unwrap_or_default();
            let text = line
                .trim_start()
                .trim_start_matches([':', '—', '–', '-'])
                .trim();
            Some((id.as_str().to_string(), text.to_string()))
        })
        .collect()
}

/// The numbered items under a done heading, as `(DONE-<n>, text)`.
///
/// `<n>` is the item's position, not the digit the author typed: Markdown
/// renders a list of `1.` lines as `1., 2., 3.`, so position is the number a
/// reader of the rendered PRD would cite. Only top-level items count — an
/// indented sub-item elaborates its parent rather than stating a new
/// obligation. The section runs until a heading of the same or a shallower
/// level, so a sub-heading inside it does not truncate the list.
fn done_items(prd: &str) -> Vec<(String, String)> {
    let mut items = Vec::new();
    let mut section_level: Option<usize> = None;
    for line in prd.lines() {
        let trimmed = line.trim_start();
        if trimmed.starts_with('#') {
            let level = trimmed.chars().take_while(|ch| *ch == '#').count();
            let inside = section_level.is_some_and(|open| level > open);
            if !inside {
                section_level = heading_states_done(trimmed).then_some(level);
            }
            continue;
        }
        if section_level.is_none() || line.len() - trimmed.len() >= 2 {
            continue;
        }
        if let Some(text) = numbered_item_text(trimmed) {
            items.push((format!("{DONE_ITEM_PREFIX}{}", items.len() + 1), text));
        }
    }
    items
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

fn heading_states_done(line: &str) -> bool {
    let lower = line.trim_start_matches('#').trim().to_ascii_lowercase();
    lower.contains("done")
        && DONE_HEADING_QUALIFIERS
            .iter()
            .any(|word| lower.contains(word))
        && !EXCLUDED_HEADINGS.iter().any(|word| lower.contains(word))
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
    let mut excluded = false;
    let mut states_obligations = None;
    for line in prd.lines() {
        let trimmed = line.trim_start();
        if trimmed.starts_with('#') {
            excluded = heading_excludes_obligations(trimmed);
            states_obligations = None;
            continue;
        }
        if !trimmed.starts_with('|') {
            states_obligations = None;
            continue;
        }
        let states = *states_obligations.get_or_insert_with(|| header_states_obligations(line));
        if excluded || !states {
            continue;
        }
        let cells = line
            .trim()
            .trim_matches('|')
            .split('|')
            .map(str::trim)
            .collect::<Vec<_>>();
        if let Some(id) = cells
            .first()
            .copied()
            .filter(|id| valid_table_obligation_id(id))
        {
            ids.push(id.to_string());
        }
    }
    ids
}

fn table_obligation_rows(prd: &str) -> Vec<(String, String)> {
    let mut rows = Vec::new();
    let mut excluded = false;
    let mut states_obligations = None;
    for line in prd.lines() {
        let trimmed = line.trim_start();
        if trimmed.starts_with('#') {
            excluded = heading_excludes_obligations(trimmed);
            states_obligations = None;
            continue;
        }
        if !trimmed.starts_with('|') {
            states_obligations = None;
            continue;
        }
        let states = *states_obligations.get_or_insert_with(|| header_states_obligations(line));
        if excluded || !states {
            continue;
        }
        let cells: Vec<_> = line
            .trim()
            .trim_matches('|')
            .split('|')
            .map(str::trim)
            .collect();
        let Some(id) = cells
            .first()
            .copied()
            .filter(|id| valid_table_obligation_id(id))
        else {
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
    let lower = line.to_ascii_lowercase();
    !EXCLUDED_HEADINGS.iter().any(|word| lower.contains(word))
        && OBLIGATION_HEADER_WORDS
            .iter()
            .any(|word| lower.contains(word))
}

/// Forbidden residual-gap phrases declared by the PRD itself.
///
/// The list begins at a heading containing "residual" and "gap", after a line
/// naming forbidden phrases, and ends at the next heading. The workflow engine
/// carries no phrase vocabulary of its own.
pub fn residual_gap_forbidden_phrases(prd: &str) -> Vec<String> {
    let mut in_gap_section = false;
    let mut in_list = false;
    let mut phrases = Vec::new();
    for line in prd.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('#') {
            let heading = trimmed.trim_start_matches('#').trim().to_ascii_lowercase();
            if in_gap_section && !(heading.contains("residual") && heading.contains("gap")) {
                break;
            }
            in_gap_section = heading.contains("residual") && heading.contains("gap");
            in_list = false;
            continue;
        }
        if !in_gap_section {
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
