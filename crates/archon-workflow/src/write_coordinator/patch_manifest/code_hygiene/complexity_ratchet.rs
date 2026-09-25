//! The complexity cap as a ratchet against the file's baseline.
//!
//! Judging only the post-patch text made a file that already held an
//! over-cap function unlandable: any patch touching it was refused for a
//! function the agent never wrote. The cap now refuses a post-patch function
//! over it only when that function is new or scores higher than it did.
//!
//! Functions are grouped by name and signature (the declaration up to the
//! body, comments dropped, literal text kept, whitespace removed), so adding,
//! removing or reordering a same-named function elsewhere (`new`, `fmt`,
//! `from`) cannot shift the pairing. Within a group, post-patch functions
//! first pair with baseline ones of equal score; each one left over is
//! judged against the group's highest remaining baseline score, and is new
//! if none remains. A group with no baseline function at all takes the
//! highest score among same-named baseline functions whose signature no
//! longer appears (an edited signature); failing that it is new. A new or
//! renamed function, or any function in a file with no baseline, is judged
//! against the cap alone.
//!
//! The gate is never weaker than the hand scanner was:
//! - a post-patch function the parser could only partly read is judged on
//!   the score of the part it read, and a refusal says so;
//! - against a baseline function the parser could only partly read, a
//!   post-patch function partly read the same way is compared score for
//!   score; a fully read one could have been measured differently, so it is
//!   excused — but only within that exact signature group;
//! - a baseline that may be missing a function (an error outside every
//!   function) is paired with the hand scanner's functions added to it, and
//!   a post-patch function with no counterpart in either is new unless its
//!   name appears as a word in the baseline text — and, when the error sits
//!   inside a container (one test case), only for functions inside that
//!   container;
//! - a baseline read under a different grammar (a `.h` resolved to C++
//!   before and C now) is held to its recovered scores, never excused;
//! - only a post-patch hand-scanner reading that lost sync skips the file,
//!   since every span after that point is suspect.
//!
//! Every unreliable reading is returned as an [`UnreliableScan`] note for
//! the caller to record.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use super::ratchet_scope::{absorbed_counterpart, is_touched, touched_lines};
use super::{
    FileScan, FunctionScore, PatchError, add_hand_functions, baseline_scan, scan_functions,
};
use crate::write_coordinator::patch_manifest::{COMPLEXITY_SCAN_UNRELIABLE, UnreliableScan};

pub(super) fn validate_complexity(
    path: &str,
    baseline: Option<&str>,
    text: &str,
    max: u32,
) -> Result<Vec<UnreliableScan>, PatchError> {
    if max == 0 {
        return Ok(Vec::new());
    }
    let mut after = scan_functions(path, text);
    let post_hand = after.parsed && after.incomplete && add_hand_functions(&mut after, path, text);
    let before = baseline.map(|text| {
        let mut scan = baseline_scan(path, text);
        if post_hand && !scan.incomplete {
            add_hand_functions(&mut scan, path, text);
        }
        scan
    });
    let mut notes = notes_for(path, "post-patch", &after);
    if let Some(before) = &before {
        notes.extend(notes_for(path, "baseline", before));
    }
    if after.lost_sync {
        return Ok(notes);
    }
    // A partly read baseline excuses only when its grammar read the same
    // way: a header read as C++ before and C now was not measured alike.
    let same_grammar = before
        .as_ref()
        .is_none_or(|scan| scan.language == after.language);
    // Only what the patch touched is judged; a new file is all touched.
    let touched = baseline.map(|baseline| touched_lines(baseline, text));
    let before = before.unwrap_or_default();
    let previous = baseline_counterparts(&before.functions, &after.functions, same_grammar);
    for (function, previous) in after.functions.into_iter().zip(previous) {
        let untouched = touched
            .as_ref()
            .is_some_and(|touched| !is_touched(&function, touched));
        if !untouched && function.score > max {
            let previous = previous.or_else(|| hidden_excuse(&function, &before, baseline));
            judge(path, function, previous, &before, max)?;
        }
    }
    Ok(notes)
}

/// A post-patch function with no counterpart is excused when the baseline
/// reading may have missed it there — an error outside every function, or in
/// a container holding it — and its name appears in the baseline text.
fn hidden_excuse(
    function: &FunctionScore,
    before: &FileScan,
    baseline: Option<&str>,
) -> Option<u32> {
    let hidden = before.incomplete
        || function
            .regions
            .iter()
            .any(|region| before.incomplete_regions.contains(region));
    baseline
        .filter(|text| hidden && named_in(&function.name, text))
        .map(|_| u32::MAX)
}

/// Judge one touched post-patch function over the cap against `previous`,
/// its same-role baseline score, or else its absorbed total against the
/// same node's baseline absorbed total.
fn judge(
    path: &str,
    function: FunctionScore,
    previous: Option<u32>,
    before: &FileScan,
    max: u32,
) -> Result<(), PatchError> {
    match previous {
        Some(was) if function.score <= was => return Ok(()),
        Some(was) => return Err(refusal(path, function, Some(was), max)),
        None => {}
    }
    match absorbed_counterpart(before, &function) {
        Some(total) if function.absorbed <= total => Ok(()),
        Some(total) => {
            let absorbed = FunctionScore {
                score: function.absorbed,
                ..function
            };
            Err(refusal(path, absorbed, Some(total), max))
        }
        None => Err(refusal(path, function, None, max)),
    }
}

/// Whether every identifier in `name` appears as a word in `text`.
fn named_in(name: &str, text: &str) -> bool {
    let words = |value: &str| -> Vec<String> {
        value
            .split(|ch: char| !(ch.is_alphanumeric() || ch == '_'))
            .filter(|word| !word.is_empty())
            .map(str::to_string)
            .collect()
    };
    if name.starts_with('<') {
        return false;
    }
    let present: BTreeSet<String> = words(text).into_iter().collect();
    let parts = words(name);
    !parts.is_empty() && parts.iter().all(|part| present.contains(part))
}

fn refusal(path: &str, function: FunctionScore, was: Option<u32>, max: u32) -> PatchError {
    if !function.reliable {
        let compared = was
            .map(|was| format!(" (was {was}, now {})", function.score))
            .unwrap_or_default();
        return PatchError::FunctionPartlyReadTooComplex {
            path: path.to_string(),
            function: function.name,
            line: function.line,
            complexity: function.score,
            max,
            compared,
        };
    }
    match was {
        Some(was) => increased(path, function, was, max),
        None => too_complex(path, function, max),
    }
}

fn notes_for(path: &str, which: &str, scan: &FileScan) -> Vec<UnreliableScan> {
    scan.unreliable
        .iter()
        .map(|(line, reason)| UnreliableScan {
            rule: COMPLEXITY_SCAN_UNRELIABLE.to_string(),
            path: path.to_string(),
            line: *line,
            language: scan.language.clone(),
            reason: format!("{which} text: {reason}"),
        })
        .collect()
}

#[derive(Default)]
struct Group {
    post: Vec<usize>,
    /// (score, read fully).
    base: Vec<(u32, bool)>,
}

/// What baseline `(score, read fully)` a post-patch function is held to.
fn held_to((score, reliable): (u32, bool), post: &FunctionScore, same_grammar: bool) -> u32 {
    if reliable || !post.reliable || !same_grammar {
        score
    } else {
        u32::MAX
    }
}

/// For each post-patch function, the baseline score it is judged against.
fn baseline_counterparts(
    before: &[FunctionScore],
    after: &[FunctionScore],
    same_grammar: bool,
) -> Vec<Option<u32>> {
    let mut groups: BTreeMap<(&str, &str, bool), Group> = BTreeMap::new();
    for (index, function) in after.iter().enumerate() {
        groups.entry(key(function)).or_default().post.push(index);
    }
    for function in before {
        let entry = (function.score, function.reliable);
        groups.entry(key(function)).or_default().base.push(entry);
    }
    let mut vanished: HashMap<(&str, bool), u32> = HashMap::new();
    for ((name, _, container), group) in &groups {
        // A partly read baseline function excuses only its own group.
        let readable = group
            .base
            .iter()
            .filter(|(_, reliable)| *reliable)
            .map(|(score, _)| *score)
            .max();
        if let (true, Some(highest)) = (group.post.is_empty(), readable) {
            let entry = vanished.entry((name, *container)).or_default();
            *entry = (*entry).max(highest);
        }
    }
    let mut out = vec![None; after.len()];
    for ((name, _, container), group) in groups {
        if group.base.is_empty() {
            for index in group.post {
                out[index] = vanished.get(&(name, container)).copied();
            }
            continue;
        }
        let mut remaining = group.base;
        let mut leftover = Vec::new();
        for index in group.post {
            let post = &after[index];
            match remaining
                .iter()
                .position(|base| held_to(*base, post, same_grammar) == post.score)
            {
                Some(at) => {
                    remaining.swap_remove(at);
                    out[index] = Some(post.score);
                }
                None => leftover.push(index),
            }
        }
        for index in leftover {
            let post = &after[index];
            out[index] = remaining
                .iter()
                .map(|base| held_to(*base, post, same_grammar))
                .max();
        }
    }
    out
}

/// Name, signature and role: a counterpart read in another role (absorbed
/// before, a container now) is matched on absorbed totals instead.
fn key(function: &FunctionScore) -> (&str, &str, bool) {
    (
        function.name.as_str(),
        function.header.as_str(),
        function.container,
    )
}

fn too_complex(path: &str, function: FunctionScore, max: u32) -> PatchError {
    PatchError::FunctionTooComplex {
        path: path.to_string(),
        function: function.name,
        line: function.line,
        complexity: function.score,
        max,
    }
}

fn increased(path: &str, function: FunctionScore, was: u32, max: u32) -> PatchError {
    PatchError::FunctionComplexityIncreased {
        path: path.to_string(),
        function: function.name,
        line: function.line,
        baseline: was,
        complexity: function.score,
        max,
    }
}
