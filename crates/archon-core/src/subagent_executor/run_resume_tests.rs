//! #241: the executor runs a resumed agent on its recorded rung and with its
//! recorded confinement, or refuses.

use super::*;
use crate::agents::transcript::{InheritedConfinement, RecordedIsolation};

fn record(isolation: RecordedIsolation, tier: IsolationTier) -> SpawnConfinement {
    SpawnConfinement {
        isolation,
        tier,
        cwd: "/work".into(),
        read_roots: Vec::new(),
        write_roots: Vec::new(),
        allowed_tools: Vec::new(),
        model: None,
        max_turns: 4,
        timeout_secs: 30,
        inherited: InheritedConfinement {
            workflow: false,
            sealed_repositories: Vec::new(),
            denied_directory_names: Vec::new(),
            parent_subagent: None,
        },
    }
}

#[test]
fn a_resume_names_its_recorded_rung_whatever_it_asked() {
    let pin = record(
        RecordedIsolation::WorkspaceBoundary,
        IsolationTier::Worktree,
    );
    assert_eq!(
        explicit_tier(Some(&pin), Some(Isolation::WorkspaceBoundary)),
        Some(IsolationTier::Worktree)
    );
    let shared = record(RecordedIsolation::Unset, IsolationTier::Shared);
    assert_eq!(
        explicit_tier(Some(&shared), None),
        Some(IsolationTier::Shared)
    );
    // A spawn has no record: its request decides, and a boundary names no rung.
    assert_eq!(
        explicit_tier(None, Some(Isolation::WorkspaceBoundary)),
        None
    );
    assert_eq!(
        explicit_tier(None, Some(Isolation::Tier(IsolationTier::Worktree))),
        Some(IsolationTier::Worktree)
    );
}

#[test]
fn a_lower_granted_rung_is_refused_and_the_recorded_one_is_not() {
    let pin = record(RecordedIsolation::Unset, IsolationTier::Worktree);
    let refusal = check_rung("a1", Some(&pin), IsolationTier::Shared).unwrap_err();
    assert!(refusal.contains("'a1'"), "{refusal}");
    assert!(refusal.contains("'worktree'"), "{refusal}");
    assert!(refusal.contains("isolation_max_tier"), "{refusal}");
    assert!(check_rung("a1", Some(&pin), IsolationTier::Worktree).is_ok());
    assert!(check_rung("a1", None, IsolationTier::Shared).is_ok());
}

#[test]
fn any_difference_from_the_record_is_refused_and_named() {
    let pin = record(RecordedIsolation::WorkspaceBoundary, IsolationTier::Shared);
    assert!(check_effective("a2", Some(&pin), &pin).is_ok());
    let mut effective = pin.clone();
    effective.cwd = "/elsewhere".into();
    effective.inherited.workflow = true;
    let refusal = check_effective("a2", Some(&pin), &effective).unwrap_err();
    assert!(refusal.contains("'a2'"), "{refusal}");
    assert!(refusal.contains("cwd, inherited"), "{refusal}");
    assert!(check_effective("a2", None, &effective).is_ok());
}
