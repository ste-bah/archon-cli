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
//! No unreliable reading refuses a patch — a scanner gap is the harness's
//! defect, not the agent's — and every one is returned as an
//! [`UnreliableScan`] note for the caller to record:
//! - the whole file is skipped when the post-patch reading lost sync (every
//!   span after that point is suspect) or the baseline reading may be missing
//!   a function (a post-patch function would then look new);
//! - a post-patch function whose own reading is unreliable is not judged;
//! - a baseline function whose own reading is unreliable is taken to have
//!   had any score, so its counterpart cannot be refused against it.

use std::collections::{BTreeMap, HashMap};

use super::{FileScan, FunctionScore, PatchError, scan_functions};
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
    let before = baseline.map(|text| scan_functions(path, text));
    let mut notes = notes_for(path, "post-patch", &after);
    if let Some(before) = &before {
        notes.extend(notes_for(path, "baseline", before));
    }
    if after.lost_sync || before.as_ref().is_some_and(|before| before.incomplete) {
        return Ok(notes);
    }
    let before = before.map(|scan| scan.functions).unwrap_or_default();
    let previous = baseline_counterparts(&before, &after.functions);
    for (function, previous) in after.functions.into_iter().zip(previous) {
        if function.score <= max || !function.reliable {
            continue;
        }
        match previous {
            Some(was) if function.score <= was => continue,
            Some(was) => return Err(increased(path, function, was, max)),
            None => return Err(too_complex(path, function, max)),
        }
    }
    Ok(notes)
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
        if let (true, Some(highest)) = (group.post.is_empty(), group.base.iter().max()) {
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
