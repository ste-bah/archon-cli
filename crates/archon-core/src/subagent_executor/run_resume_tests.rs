//! A stored rung is used directly; only the current cap is consulted.
use super::*;

#[test]
fn a_lower_granted_rung_is_refused_and_the_recorded_one_is_not() {
    let refusal = check_rung("a1", IsolationTier::Worktree, IsolationTier::Shared).unwrap_err();
    assert!(
        refusal.contains("'a1'")
            && refusal.contains("worktree")
            && refusal.contains("isolation_max_tier")
    );
    assert!(check_rung("a1", IsolationTier::Worktree, IsolationTier::Worktree).is_ok());
    assert!(
        check_rung(
            "a1",
            IsolationTier::Worktree,
            IsolationTier::WorktreeWithBuilds
        )
        .is_ok()
    );
}

#[test]
fn the_build_rung_cannot_be_lowered_to_a_plain_worktree() {
    assert!(
        check_rung(
            "a",
            IsolationTier::WorktreeWithBuilds,
            IsolationTier::Worktree
        )
        .is_err()
    );
    assert!(
        check_rung(
            "a",
            IsolationTier::WorktreeWithBuilds,
            IsolationTier::WorktreeWithBuilds
        )
        .is_ok()
    );
}

#[test]
fn a_shared_agent_still_has_a_known_rung() {
    assert!(check_rung("a", IsolationTier::Shared, IsolationTier::Shared).is_ok());
}
