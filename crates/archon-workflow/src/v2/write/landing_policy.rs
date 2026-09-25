//! The landing policies the patch validator enforces, told to the coder
//! before it writes (Issue-52).
//!
//! # The gap this closes
//!
//! `write_coordinator::patch_manifest::validate_patch` refuses a captured
//! patch when any changed source file exceeds `max_source_file_lines`, any
//! function in one exceeds `max_function_complexity`, any changed file
//! exceeds `max_file_bytes` or the patch exceeds `max_patch_bytes`. One
//! violation refuses the whole patch, and the check runs after the session
//! has returned. Nothing in the coder's prompt named those limits, so live
//! one task lost three consecutive attempts — about four hours — learning
//! them one rejection at a time: first the line cap, then the complexity
//! cap, the whole patch discarded each time.
//!
//! # One config, two readers
//!
//! The numbers here are read from the same [`WriteCoordinatorConfig`] the
//! validator is handed, and the counting rules are quoted from the
//! enforcer's own constants (`code_hygiene::CHECKED_SOURCE_EXTENSIONS`,
//! `BRANCH_TOKENS`, `LOGICAL_OPERATORS`), so the prompt describes the metric
//! the gate computes rather than a textbook one. The per-target headroom
//! line reads the `target_file_budgets` stamp `target_budgets` already
//! writes onto the item, restated in words so the coder sees how much of
//! each declared file is already spent.
//!
//! The section is appended to the branch's rendered task after the
//! forbidden-paths sentence, and BEFORE the task is kept as the retry and
//! restart base, so every write-capable dispatch — first session, in-run
//! retry, mid-attempt restart — carries it.

use serde_json::Value;

use crate::write_coordinator::WriteCoordinatorConfig;
use crate::write_coordinator::patch_manifest::code_hygiene::{
    BRANCH_TOKENS, CHECKED_SOURCE_EXTENSIONS, LOGICAL_OPERATORS,
};

/// The section appended to the branch's task. `item` is the rendered item
/// object (`input.item`), read for its `target_file_budgets` stamp; the
/// headroom line is omitted when no budgets were stamped.
pub(super) fn preamble(cfg: &WriteCoordinatorConfig, item: &Value) -> String {
    let mut text = String::from(
        "\nLanding policy (enforced on the captured patch after you return; a single violation \
         refuses the ENTIRE patch — every file in it, however correct — so split files into \
         submodules and functions into helpers BEFORE returning):\n",
    );
    text.push_str(&line_cap_rule(cfg.max_source_file_lines));
    text.push_str(&complexity_rule(cfg.max_function_complexity));
    text.push_str(&format!(
        "- File size: at most {} bytes per changed file, whatever its extension.\n",
        cfg.max_file_bytes
    ));
    text.push_str(&format!(
        "- Patch size: at most {} bytes for the whole patch.\n",
        cfg.max_patch_bytes
    ));
    if let Some(headroom) = headroom(item) {
        text.push_str(&headroom);
    }
    text
}

/// The line cap as `code_hygiene::validate_line_count` applies it: every
/// line of the post-edit file counts, only the listed extensions are
/// checked, and a file already over the cap is tolerated while it does not
/// grow. A cap of zero disables the check there, so it is said to here.
fn line_cap_rule(max: u32) -> String {
    if max == 0 {
        return "- Source file length: no cap is configured.\n".to_string();
    }
    format!(
        "- Source file length: at most {max} lines per changed file, counted as every line of \
         the file after your edit — blank lines, comments, doc comments and in-file test \
         modules all count. Applies to changed files with one of these extensions: {}. A file \
         already over the cap may be changed only if it does not grow; put new code in a new \
         file under the module directory you own and re-export it.\n",
        CHECKED_SOURCE_EXTENSIONS.join(", ")
    )
}

/// The complexity metric as `code_hygiene::function_scores` computes it: 1
/// per function, plus one per branch token and one per logical operator on
/// every line from the signature to the closing brace (or, for `def`
/// blocks, to the dedent), comment tails stripped. Like the line cap it is a
/// ratchet (`code_hygiene::complexity_ratchet`): a function already over
/// the cap in the baseline is refused only if its score grows.
fn complexity_rule(max: u32) -> String {
    if max == 0 {
        return "- Function complexity: no cap is configured.\n".to_string();
    }
    format!(
        "- Function complexity: at most {max} per function, scored as 1 plus one for every {} \
         token and one for every {} operator, counted on every line of the function from its \
         signature to its closing brace (nested closures and blocks included; comment text \
         after `//` or `#` excluded). A function already over the cap may be changed only if \
         its score does not grow; a renamed function counts as new.\n",
        backticked(BRANCH_TOKENS),
        backticked(LOGICAL_OPERATORS)
    )
}

/// `a`, `b` or `c` — the enforcer's list quoted for a reader.
fn backticked(items: &[&str]) -> String {
    let quoted: Vec<String> = items.iter().map(|item| format!("`{item}`")).collect();
    match quoted.split_last() {
        Some((last, [])) => last.clone(),
        Some((last, rest)) => format!("{} or {last}", rest.join(", ")),
        None => String::new(),
    }
}

/// One line naming, for each declared target the budget stamp measured,
/// how many of its lines are already used. `None` when the item carries no
/// stamp (no repository root, or no declared files).
fn headroom(item: &Value) -> Option<String> {
    let budgets = item.get("target_file_budgets")?.as_array()?;
    let entries: Vec<String> = budgets.iter().filter_map(budget_sentence).collect();
    if entries.is_empty() {
        return None;
    }
    Some(format!(
        "Declared target headroom: {}.\n",
        entries.join("; ")
    ))
}

/// `path: N of M lines used (R remaining)`, or the over-cap wording when
/// nothing remains — the stamp's own numbers, restated.
fn budget_sentence(budget: &Value) -> Option<String> {
    let path = budget.get("path")?.as_str()?;
    let used = budget.get("current_lines")?.as_u64()?;
    let cap = budget.get("max_lines")?.as_u64()?;
    let remaining = budget
        .get("lines_remaining")
        .and_then(Value::as_u64)
        .unwrap_or_else(|| cap.saturating_sub(used));
    Some(if used >= cap {
        format!("{path}: {used} of {cap} lines used (at or over the cap; it may not grow)")
    } else {
        format!("{path}: {used} of {cap} lines used ({remaining} remaining)")
    })
}

#[cfg(test)]
#[path = "landing_policy_tests.rs"]
mod tests;
