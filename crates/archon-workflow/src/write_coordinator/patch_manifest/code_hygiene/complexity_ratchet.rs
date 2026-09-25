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
//! - a post-patch function whose syntax tree holds an error is judged on
//!   its recovered score, and a refusal says it holds a syntax error;
//! - a baseline function whose own reading is unreliable is taken to have
//!   had any score, but only for its own exact signature group;
//! - a baseline that may be missing a function (a syntax error outside every
//!   function) is read by the hand scanner instead when that stays in sync;
//!   failing that, a post-patch function without a counterpart is new
//!   unless its name appears as a word in the baseline text;
//! - only a post-patch hand-scanner reading that lost sync skips the file,
//!   since every span after that point is suspect.
//!
//! Every unreliable reading is returned as an [`UnreliableScan`] note for
//! the caller to record.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use super::{FileScan, FunctionScore, PatchError, baseline_scan, scan_functions};
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
    let after = scan_functions(path, text);
    let before = baseline.map(|text| baseline_scan(path, text));
    let mut notes = notes_for(path, "post-patch", &after);
    if let Some(before) = &before {
        notes.extend(notes_for(path, "baseline", before));
    }
    if after.lost_sync {
        return Ok(notes);
    }
    let partial = baseline.filter(|_| before.as_ref().is_some_and(|scan| scan.incomplete));
    let before = before.map(|scan| scan.functions).unwrap_or_default();
    let previous = baseline_counterparts(&before, &after.functions);
    for (function, previous) in after.functions.into_iter().zip(previous) {
        let previous = previous.or_else(|| {
            partial
                .filter(|text| named_in(&function.name, text))
                .map(|_| u32::MAX)
        });
        if function.score <= max {
            continue;
        }
        match previous {
            Some(was) if function.score <= was => continue,
            was => return Err(refusal(path, function, was, max)),
        }
    }
    Ok(notes)
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
        return PatchError::FunctionWithSyntaxErrorTooComplex {
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
    base: Vec<u32>,
}

/// For each post-patch function, the baseline score it is judged against.
fn baseline_counterparts(before: &[FunctionScore], after: &[FunctionScore]) -> Vec<Option<u32>> {
    let mut groups: BTreeMap<(&str, &str), Group> = BTreeMap::new();
    for (index, function) in after.iter().enumerate() {
        groups.entry(key(function)).or_default().post.push(index);
    }
    for function in before {
        // An unreliable baseline reading could have been any score.
        let score = if function.reliable {
            function.score
        } else {
            u32::MAX
        };
        groups.entry(key(function)).or_default().base.push(score);
    }
    let mut vanished: HashMap<&str, u32> = HashMap::new();
    for ((name, _), group) in &groups {
        // An unreliable baseline reading excuses only its own group.
        let readable = group.base.iter().filter(|score| **score != u32::MAX).max();
        if let (true, Some(highest)) = (group.post.is_empty(), readable) {
            let entry = vanished.entry(name).or_default();
            *entry = (*entry).max(*highest);
        }
    }
    let mut out = vec![None; after.len()];
    for ((name, _), group) in groups {
        if group.base.is_empty() {
            for index in group.post {
                out[index] = vanished.get(name).copied();
            }
            continue;
        }
        let mut remaining = group.base;
        let mut leftover = Vec::new();
        for index in group.post {
            let score = after[index].score;
            match remaining.iter().position(|was| *was == score) {
                Some(at) => {
                    remaining.swap_remove(at);
                    out[index] = Some(score);
                }
                None => leftover.push(index),
            }
        }
        let highest = remaining.iter().max().copied();
        for index in leftover {
            out[index] = highest;
        }
    }
    out
}

fn key(function: &FunctionScore) -> (&str, &str) {
    (function.name.as_str(), function.header.as_str())
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
