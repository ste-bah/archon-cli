//! Reading one item out of a markdown list, and the items under a heading.

use super::sorted_unique;

/// The text of a markdown list item, whichever list marker was used.
///
/// Only `- ` and `* ` were accepted. Every task in the live PRD writes its
/// acceptance criteria as a numbered list, so all fifteen parsed to zero
/// criteria. `pin_noop_acceptance_criteria` then stamped an empty array, the
/// no-op proof rejected it as "acceptance_criteria is missing or empty", and
/// the repair loop could never satisfy a field no agent controls — twelve
/// identical iterations before the run halted.
pub(super) fn list_item_text(trimmed: &str) -> Option<&str> {
    if let Some(rest) = trimmed
        .strip_prefix("- ")
        .or_else(|| trimmed.strip_prefix("* "))
        .or_else(|| trimmed.strip_prefix("+ "))
    {
        return Some(rest.trim());
    }
    // `1. criterion`, `12) criterion` — the ordinal is the marker, not content.
    let digits = trimmed.len()
        - trimmed
            .trim_start_matches(|c: char| c.is_ascii_digit())
            .len();
    if digits == 0 {
        return None;
    }
    let rest = &trimmed[digits..];
    rest.strip_prefix(". ")
        .or_else(|| rest.strip_prefix(") "))
        .map(str::trim)
}

/// List items under a heading, each JOINED ACROSS THE LINES IT WRAPS ONTO.
///
/// A bullet that wraps is one item, and this used to keep only its first line:
/// a continuation line is not a list item, so it matched nothing and was
/// dropped in silence. Every criterion long enough to wrap reached the runtime
/// cut off mid-sentence.
///
/// That is not cosmetic. On a reference corpus one criterion read, in full:
/// "The ingest has been executed and `<registry>` ... lists a dataset for each
/// of the thirty cells ... Compiling code with an empty data lake does not
/// satisfy this task." The runtime saw it end at "`<registry>`". The clause
/// saying what must be IN the registry, and the sentence explicitly refusing
/// the exact shortcut that was taken, were both discarded before any gate could
/// read them — so runs closed the task on the strength of files existing, and
/// every stricter acceptance rule slid off a demand it could not see.
///
/// A blank line ends an item: it separates bullets in every markdown flavour,
/// and without it a trailing paragraph would be swallowed into the last one.
pub(super) fn declared_task_section_items(raw: &str, section: &str) -> Vec<String> {
    let mut items: Vec<String> = Vec::new();
    let mut in_section = false;
    let mut current: Option<String> = None;
    for line in raw.lines() {
        let trimmed = line.trim();
        if let Some(heading) = trimmed.strip_prefix('#') {
            push_section_item(&mut items, current.take());
            in_section = heading_matches_section(heading.trim_start_matches('#'), section);
            continue;
        }
        if !in_section {
            continue;
        }
        if trimmed.is_empty() {
            push_section_item(&mut items, current.take());
            continue;
        }
        if let Some(item) = list_item_text(trimmed) {
            push_section_item(&mut items, current.take());
            current = Some(item.to_string());
        } else if let Some(open) = current.as_mut() {
            // A continuation of the bullet above. Joined with a space because
            // the newline it replaces is a wrap, not a separator.
            open.push(' ');
            open.push_str(trimmed);
        }
    }
    push_section_item(&mut items, current);
    sorted_unique(items)
}

pub(super) fn push_section_item(items: &mut Vec<String>, item: Option<String>) {
    if let Some(item) = item {
        let item = item.trim();
        if !item.is_empty() {
            items.push(item.to_string());
        }
    }
}

/// The words of a heading, lowercased, punctuation dropped.
///
/// Comparing headings as whole strings makes every difference fatal, including
/// differences that carry no meaning — trailing punctuation, doubled spaces,
/// case. Comparing word sequences keeps the ordering that does carry meaning
/// while discarding the decoration that does not.
fn heading_words(text: &str) -> Vec<String> {
    text.split_whitespace()
        .map(|word| {
            word.chars()
                .filter(|c| c.is_alphanumeric())
                .flat_map(char::to_lowercase)
                .collect::<String>()
        })
        .filter(|word| !word.is_empty())
        .collect()
}

/// Whether a heading opens the requested section.
///
/// Exact equality was the rule, and it is the third silent-empty defect in this
/// file. On one reference corpus the parser asked for `files expected to
/// change` while the task files wrote `Files Expected` (four files) and `Files
/// Expected to Change During Implementation` (two). Six of fifteen tasks
/// therefore declared no target files at all, and every consumer — the wave
/// planner's write-conflict detection among them — read that as "this task
/// changes nothing" rather than "the section was not found".
///
/// A heading matches when its words are a prefix of the section's, or the
/// section's are a prefix of the heading's. That accepts an abbreviation and an
/// elaboration of the same section name without naming either spelling here, so
/// no per-project heading table is introduced and no future variant of the same
/// shape needs a code change. Prefixes are compared word-wise, never as raw
/// strings, so `files expected` cannot match `files expectedly`.
pub(super) fn heading_matches_section(heading: &str, section: &str) -> bool {
    let heading = heading_words(heading);
    let section = heading_words(section);
    if heading.is_empty() || section.is_empty() {
        return false;
    }
    heading.starts_with(section.as_slice()) || section.starts_with(heading.as_slice())
}

/// Words carrying no distinguishing meaning in a heading.
///
/// Deliberately ordinary English function words. A heading is a phrase, not a
/// sentence, so this list stays short; the point is only that "to" and "during"
/// must not make two headings look different when the nouns agree.
const HEADING_STOPWORDS: &[&str] = &[
    "a", "an", "and", "at", "by", "during", "for", "in", "of", "on", "the", "to", "with",
];

/// A word with a common English inflection removed.
///
/// Applied ONLY when looking for a near miss, never when deciding a match. A
/// heading that opens a section must say the section's words; a heading merely
/// *reported* as resembling one may inflect them, and `Files Expected to Be
/// Changed` plainly means the same section as `Files Expected to Change`
/// despite sharing no whole word beyond the first two.
///
/// Applied repeatedly, because one pass does not converge: `changed` loses
/// `ed` to give `chang`, while `change` keeps its trailing `e` and the two
/// never meet. Stripping until nothing more comes off takes both to `chang`,
/// and takes `file` and `files` both to `file` — the floor stops the second
/// pass there rather than reducing one of them further than the other.
///
/// Kept to the suffixes that carry no meaning in a heading, and only on words
/// long enough that removing one leaves something: `used` must not become `us`.
fn stem(word: &str) -> &str {
    let mut word = word;
    loop {
        // Every suffix is tried, not just the first that strips: `files` loses
        // `es` to leave `fil`, which is below the floor, and the answer is the
        // `s` behind it rather than no stem at all.
        let Some(root) = ["ing", "ed", "es", "s", "e"]
            .iter()
            .filter_map(|suffix| word.strip_suffix(suffix))
            .find(|root| root.len() >= 4)
        else {
            return word;
        };
        word = root;
    }
}

fn significant_words(words: &[String]) -> Vec<&str> {
    words
        .iter()
        .filter(|word| !HEADING_STOPWORDS.contains(&word.as_str()))
        .map(|word| stem(word))
        .collect()
}

/// Headings that plainly mean the requested section but did not match it.
///
/// The durable half of the fix. Widening the match rule handles the variants
/// seen so far; it cannot handle the next one, and this file's history is that
/// there is always a next one — list markers, then wrapped bullets, now
/// headings, each found only after a run had already been ruined by it.
///
/// What made all three expensive was not the mismatch. It was that a section
/// which failed to match and a section that was genuinely empty arrived at
/// every consumer as the same empty list, so nothing downstream could tell a
/// task that declares nothing from a task whose declaration was dropped. This
/// reports the difference instead of erasing it.
///
/// A heading is a near miss when the significant words of one are a subset of
/// the other's and at least two are shared — enough to catch a reordering or a
/// renamed qualifier, while `Files Forbidden` stays clear of `Files Expected to
/// Change` because it shares only one.
pub(super) fn heading_near_misses(raw: &str, section: &str) -> Vec<String> {
    let section_words = heading_words(section);
    let section_significant = significant_words(&section_words);
    let mut misses = Vec::new();
    for line in raw.lines() {
        let trimmed = line.trim();
        let Some(heading) = trimmed.strip_prefix('#') else {
            continue;
        };
        let heading = heading.trim_start_matches('#').trim();
        if heading_matches_section(heading, section) {
            // The section was found. Nothing to report, whichever spelling won.
            return Vec::new();
        }
        let heading_words = heading_words(heading);
        let heading_significant = significant_words(&heading_words);
        let shared = heading_significant
            .iter()
            .filter(|word| section_significant.contains(*word))
            .count();
        let subset = shared == heading_significant.len() || shared == section_significant.len();
        if shared >= 2 && subset {
            misses.push(heading.to_string());
        }
    }
    sorted_unique(misses)
}

#[cfg(test)]
#[path = "task_universe_wrapped_criteria_tests.rs"]
mod wrapped_criteria_tests;

#[cfg(test)]
#[path = "task_universe_heading_tests.rs"]
mod heading_tests;

/// Focused-test commands a task declares, from bullets and fenced blocks alike.
///
/// `declared_task_section_items` reads list items only, so a body that writes
/// its commands in a ```sh block declared nothing. The universe then recorded
/// no focused tests, the v3 author brief said "no task declares any focused
/// test", and the authored workflow passed no focusedTests at all — every task
/// verified more weakly than its own body specified.
///
/// Kept separate from the generic section reader because that one also feeds
/// acceptance criteria and the file lists, where a fenced block means something
/// else.
pub(super) fn declared_focused_tests(raw: &str) -> Vec<String> {
    // Subheadings separate checks but do not end their parent section.
    let mut depth = None;
    let mut fenced = false;
    let mut flattened = String::new();
    for line in raw.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("```") { fenced = !fenced; }
        if !fenced && trimmed.starts_with('#') {
            let level = trimmed.chars().take_while(|c| *c == '#').count();
            if depth.is_some_and(|parent| level > parent) {
                flattened.push('\n');
                continue;
            }
            depth = heading_matches_section(trimmed.trim_start_matches('#'), "focused tests")
                .then_some(level);
        }
        flattened.push_str(line);
        flattened.push('\n');
    }
    let mut items = declared_task_section_items(&flattened, "focused tests");
    items.extend(fenced_section_commands(&flattened, "focused tests"));
    sorted_unique(items)
}

/// Non-empty, non-comment lines inside fenced blocks under one section.
fn fenced_section_commands(raw: &str, section: &str) -> Vec<String> {
    let mut commands = Vec::new();
    let mut in_section = false;
    let mut in_fence = false;
    for line in raw.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("```") {
            // A fence only toggles inside the section, so a stray closing fence
            // elsewhere cannot switch this reader on.
            if in_section {
                in_fence = !in_fence;
            }
            continue;
        }
        if !in_fence && let Some(heading) = trimmed.strip_prefix('#') {
            in_section = heading_matches_section(heading.trim_start_matches('#'), section);
            continue;
        }
        if !in_section || !in_fence || trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        commands.push(trimmed.to_string());
    }
    commands
}
