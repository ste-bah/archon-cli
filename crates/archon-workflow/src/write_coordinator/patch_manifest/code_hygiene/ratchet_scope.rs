//! What the ratchet judges, and against what, when readings shift under an
//! edit.
//!
//! - **Diff scoping.** Only a post-patch function whose lines the patch
//!   added, changed or cut into is judged: a function whose every line is
//!   unchanged cannot have got worse by itself, however an edit elsewhere
//!   reshaped the reading around it (a callback turning into a container
//!   when a second handler is added beside it, a receiver renamed on another
//!   line). Moved lines are unchanged: the diff is a proper line diff.
//! - **Absorbed totals.** Every function-like node also carries what it
//!   scores as an absorber — its own lines and every nested callback, named
//!   functions left out. When a judged function has no baseline counterpart
//!   in the same role (it was absorbed into another before, or absorbed
//!   others), its absorbed total is compared with the absorbed total of the
//!   same node in the baseline, matched by name and signature or, when a
//!   receiver was renamed (`db.transaction` to `database.transaction`), by
//!   signature and the callee's final member. It is refused only if that
//!   total grew over the cap, or when nothing matches and it is over the cap.

use similar::{Algorithm, DiffOp, capture_diff_slices};

use super::{FileScan, FunctionScore};

/// Per 1-based post-patch line, whether the patch touched it.
pub(super) fn touched_lines(baseline: &str, post: &str) -> Vec<bool> {
    let old: Vec<&str> = baseline.lines().collect();
    let new: Vec<&str> = post.lines().collect();
    let mut touched = vec![false; new.len() + 2];
    for op in capture_diff_slices(Algorithm::Myers, &old, &new) {
        match op {
            DiffOp::Equal { .. } => {}
            DiffOp::Insert {
                new_index, new_len, ..
            }
            | DiffOp::Replace {
                new_index, new_len, ..
            } => {
                for line in new_index..new_index + new_len {
                    touched[line + 1] = true;
                }
            }
            // Lines cut between post lines `new_index` and `new_index + 1`
            // (1-based): both neighbours were touched.
            DiffOp::Delete { new_index, .. } => {
                touched[new_index] = true;
                touched[(new_index + 1).min(new.len() + 1)] = true;
            }
        }
    }
    touched
}

/// Whether any line of `function` was touched.
pub(super) fn is_touched(function: &FunctionScore, touched: &[bool]) -> bool {
    let end = function.end_line.max(function.line);
    (function.line..=end).any(|line| touched.get(line).copied().unwrap_or(true))
}

/// The highest baseline absorbed total of the node `function` is, if any:
/// the same name and signature read in another role (absorbed into another
/// function, or the other of container and absorber) — a same-role reading
/// was already paired by score — or, failing that, a differently named node
/// with the same signature and callee member (a renamed receiver).
pub(super) fn absorbed_counterpart(before: &FileScan, function: &FunctionScore) -> Option<u32> {
    let judged_elsewhere = before
        .functions
        .iter()
        .filter(|node| node.container != function.container);
    let same = judged_elsewhere
        .chain(&before.nodes)
        .filter(|node| node.name == function.name && node.header == function.header)
        .map(|node| node.absorbed)
        .max();
    same.or_else(|| {
        let member = final_member(&function.name)?;
        before
            .functions
            .iter()
            .chain(&before.nodes)
            .filter(|node| {
                node.name != function.name
                    && node.header == function.header
                    && final_member(&node.name) == Some(member)
            })
            .map(|node| node.absorbed)
            .max()
    })
}

/// `transaction` for a callback named `database.transaction(...)`; `None`
/// for a declared name, which has no receiver to rename.
fn final_member(name: &str) -> Option<&str> {
    let (callee, _) = name.split_once('(')?;
    Some(callee.rsplit('.').next().unwrap_or(callee))
}
