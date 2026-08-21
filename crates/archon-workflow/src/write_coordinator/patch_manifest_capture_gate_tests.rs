//! The capture gate that actually discarded correct work.
//!
//! A write branch clears three ownership gates. An earlier attempt at the scope
//! extension relaxed only the adapter's, passed 13 tests, and changed nothing —
//! because this gate, `validated_workspace_changes` inside `capture_patch`,
//! still raised `UndeclaredWrite`. These are the pair that would have caught
//! that: the same capture, refused with the plan as declared and accepted once
//! the plan is widened.
//!
//! Split from `patch_manifest_tests.rs` for the 500-line ceiling; the fixtures
//! are that file's.

use super::tests::{isolate, plan_for};
use super::capture_patch;

/// Today's behaviour, pinned. Correct work, one unlisted path, discarded.
#[test]
fn an_undeclared_change_is_refused_by_the_capture_gate() {
    let (_repo, plan, ws, baseline) = isolate(&["src/lib.rs"]);
    std::fs::write(plan.isolated_root.join("src/lib.rs"), "// edited\n").expect("edit");
    std::fs::write(plan.isolated_root.join("src/forgotten.rs"), "// also\n").expect("edit");

    let err = capture_patch(&ws, &plan.target_files, &baseline)
        .expect_err("an undeclared write must be refused");
    assert!(
        err.to_string().contains("forgotten.rs"),
        "the refusal must name the path: {err}"
    );
}

/// The fix, proved at the gate that mattered: widening the PLAN — not the
/// adapter's copy of the scope — lets the same capture through, and the granted
/// path is hashed as a declared target so the apply-time overlap and
/// stale-baseline guards cover it.
#[test]
fn widening_the_plan_lets_the_same_capture_through() {
    let (repo, plan, _ws, baseline) = isolate(&["src/lib.rs"]);
    std::fs::write(plan.isolated_root.join("src/lib.rs"), "// edited\n").expect("edit");
    std::fs::write(plan.isolated_root.join("src/forgotten.rs"), "// also\n").expect("edit");

    let widened = plan_for(repo.path(), &["src/lib.rs", "src/forgotten.rs"]);
    let ws = crate::write_coordinator::worktree_isolation::ItemWorkspace {
        plan: widened.clone(),
        baseline_commit: _ws.baseline_commit.clone(),
    };
    let captured =
        capture_patch(&ws, &widened.target_files, &baseline).expect("widened plan captures");

    assert!(
        captured.post_hashes.contains_key("src/forgotten.rs"),
        "granted path must be hashed as declared, or apply-time guards cannot see it: {:?}",
        captured.post_hashes.keys().collect::<Vec<_>>()
    );
}
