//! The complexity cap as a ratchet against the file's baseline.
//!
//! Judging only the post-patch text made a file that already held an
//! over-cap function unlandable: any patch touching it was refused for a
//! function the agent never wrote. The cap now refuses a post-patch function
//! over it only when that function is new or scores higher than it did.
//!
//! A post-patch function is matched to the baseline function with the same
//! name and the same occurrence index (the second `new` to the second
//! `new`), so duplicate names across `impl` blocks pair up in file order.
//! A renamed function has no counterpart and is judged as new. A file with
//! no baseline has every function judged as new.

use std::collections::HashMap;

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
    let before = baseline_scores(path, baseline);
    let mut seen: HashMap<String, usize> = HashMap::new();
    for function in function_scores(path, text) {
        let occurrence = seen.entry(function.name.clone()).or_default();
        let previous = before
            .get(&function.name)
            .and_then(|scores| scores.get(*occurrence))
            .copied();
        *occurrence += 1;
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

/// Baseline scores per function name, in file order.
fn baseline_scores(path: &str, baseline: Option<&str>) -> HashMap<String, Vec<u32>> {
    let mut out: HashMap<String, Vec<u32>> = HashMap::new();
    for function in baseline
        .map(|text| function_scores(path, text))
        .unwrap_or_default()
    {
        out.entry(function.name).or_default().push(function.score);
    }
    out
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
