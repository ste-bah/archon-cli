//! The complexity cap as a ratchet against the file's baseline.
//!
//! Judging only the post-patch text made a file that already held an
//! over-cap function unlandable: any patch touching it was refused for a
//! function the agent never wrote. The cap now refuses a post-patch function
//! over it only when that function is new or scores higher than it did.
//!
//! Pairing post-patch functions with baseline ones, per name:
//! 1. same name and same normalized header text (the declaration up to the
//!    body, whitespace removed), exact duplicates taken in file order — so
//!    adding or removing a same-named function elsewhere (`new`, `fmt`,
//!    `from`) does not shift the pairing;
//! 2. the rest, when as many remain on both sides (signatures edited), in
//!    file order;
//! 3. otherwise (functions of that name were added or removed as well as
//!    edited) every remaining post-patch one is compared with the highest
//!    remaining baseline score of that name: it cannot be told which one it
//!    was, so none is refused for a score some baseline one already had.
//!
//! A function with no counterpart — a new or renamed one, or any function
//! in a file with no baseline — is judged against the cap alone.

use std::collections::BTreeSet;

use super::{FunctionScore, PatchError, function_scores};

pub(super) fn validate_complexity(
    path: &str,
    baseline: Option<&str>,
    text: &str,
    max: u32,
) -> Result<(), PatchError> {
    if max == 0 {
        return Ok(());
    }
    let after = function_scores(path, text);
    let before = baseline
        .map(|text| function_scores(path, text))
        .unwrap_or_default();
    let previous = baseline_counterparts(&before, &after);
    for (function, previous) in after.into_iter().zip(previous) {
        if function.score <= max {
            continue;
        }
        match previous {
            Some(was) if function.score <= was => continue,
            Some(was) => return Err(increased(path, function, was, max)),
            None => return Err(too_complex(path, function, max)),
        }
    }
    Ok(())
}

/// For each post-patch function, the baseline score it is judged against.
fn baseline_counterparts(before: &[FunctionScore], after: &[FunctionScore]) -> Vec<Option<u32>> {
    let mut out = vec![None; after.len()];
    let names: BTreeSet<&str> = after
        .iter()
        .map(|function| function.name.as_str())
        .collect();
    for name in names {
        let post: Vec<usize> = (0..after.len())
            .filter(|index| after[*index].name == name)
            .collect();
        let base: Vec<&FunctionScore> = before
            .iter()
            .filter(|function| function.name == name)
            .collect();
        pair_one_name(&post, &base, after, &mut out);
    }
    out
}

fn pair_one_name(
    post: &[usize],
    base: &[&FunctionScore],
    after: &[FunctionScore],
    out: &mut [Option<u32>],
) {
    let mut used = vec![false; base.len()];
    let mut unmatched = Vec::new();
    for &index in post {
        let exact = (0..base.len()).find(|b| !used[*b] && base[*b].header == after[index].header);
        match exact {
            Some(b) => {
                used[b] = true;
                out[index] = Some(base[b].score);
            }
            None => unmatched.push(index),
        }
    }
    let rest: Vec<u32> = base
        .iter()
        .zip(&used)
        .filter(|(_, used)| !**used)
        .map(|(function, _)| function.score)
        .collect();
    if rest.len() == unmatched.len() {
        for (index, score) in unmatched.into_iter().zip(rest) {
            out[index] = Some(score);
        }
    } else if let Some(highest) = rest.iter().copied().max() {
        for index in unmatched {
            out[index] = Some(highest);
        }
    }
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
