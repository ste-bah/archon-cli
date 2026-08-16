//! Isolation ladder tests (#184 M3).

use super::*;

fn req(explicit: Option<&str>, overlaps: bool, write_capable: bool) -> IsolationRequest {
    IsolationRequest {
        explicit: explicit.map(str::to_string),
        overlaps_live_claim: overlaps,
        write_capable,
    }
}

const NO_CAP: IsolationTier = IsolationTier::WorktreeWithBuilds;

/// The default that keeps the disk bill at zero: disjoint writers share the
/// tree, and M2's claims are what stop them colliding.
#[test]
fn disjoint_writers_share_the_tree() {
    let (tier, reason) = resolve_tier(&req(None, false, true), AutoIsolation::Overlap, NO_CAP);
    assert_eq!(tier, IsolationTier::Shared);
    assert_eq!(reason, IsolationReason::Default);
}

/// Overlap is the only automatic trigger. Isolation costs disk, so it is spent
/// where there is an actual conflict rather than on every parallel spawn.
#[test]
fn an_overlapping_writer_is_isolated() {
    let (tier, reason) = resolve_tier(&req(None, true, true), AutoIsolation::Overlap, NO_CAP);
    assert_eq!(tier, IsolationTier::Worktree);
    assert_eq!(reason, IsolationReason::OverlappingClaim);
}

/// A read-only agent cannot conflict with anyone, so it never earns a worktree
/// however the policy is set.
#[test]
fn a_read_only_agent_is_never_isolated() {
    for auto in [
        AutoIsolation::Off,
        AutoIsolation::Overlap,
        AutoIsolation::Always,
    ] {
        let (tier, _) = resolve_tier(&req(None, true, false), auto, NO_CAP);
        assert_eq!(tier, IsolationTier::Shared, "auto = {auto:?}");
    }
}

#[test]
fn auto_isolation_off_never_isolates_on_its_own() {
    let (tier, reason) = resolve_tier(&req(None, true, true), AutoIsolation::Off, NO_CAP);
    assert_eq!(tier, IsolationTier::Shared);
    assert_eq!(reason, IsolationReason::Default);
}

#[test]
fn auto_isolation_always_isolates_every_writer() {
    let (tier, reason) = resolve_tier(&req(None, false, true), AutoIsolation::Always, NO_CAP);
    assert_eq!(tier, IsolationTier::Worktree);
    assert_eq!(reason, IsolationReason::PolicyAlways);
}

#[test]
fn an_explicit_request_wins_over_the_automatic_decision() {
    let (tier, reason) = resolve_tier(
        &req(Some("worktree-with-builds"), false, true),
        AutoIsolation::Off,
        NO_CAP,
    );
    assert_eq!(tier, IsolationTier::WorktreeWithBuilds);
    assert_eq!(reason, IsolationReason::Requested);
}

/// Asking for `none` is honoured. An agent that knows it is disjoint should be
/// able to say so, and the cost of being wrong is a conflict, not a disk fire.
#[test]
fn an_explicit_none_opts_out_even_when_claims_overlap() {
    let (tier, reason) = resolve_tier(
        &req(Some("none"), true, true),
        AutoIsolation::Always,
        NO_CAP,
    );
    assert_eq!(tier, IsolationTier::Shared);
    assert_eq!(reason, IsolationReason::Requested);
}

/// The cap is what makes the expensive rung unreachable until an operator
/// allows it, and a clamp says so rather than downgrading silently.
#[test]
fn the_cap_clamps_and_reports_what_was_asked_for() {
    let (tier, reason) = resolve_tier(
        &req(Some("worktree-with-builds"), false, true),
        AutoIsolation::Off,
        IsolationTier::Worktree,
    );
    assert_eq!(tier, IsolationTier::Worktree);
    assert_eq!(
        reason,
        IsolationReason::Clamped(IsolationTier::WorktreeWithBuilds)
    );
}

#[test]
fn a_cap_of_shared_disables_isolation_entirely() {
    let (tier, reason) = resolve_tier(
        &req(Some("worktree"), true, true),
        AutoIsolation::Always,
        IsolationTier::Shared,
    );
    assert_eq!(tier, IsolationTier::Shared);
    assert_eq!(reason, IsolationReason::Clamped(IsolationTier::Worktree));
}

/// An unrecognised value is not silently honoured as some tier — it falls
/// through to the automatic decision, which is the safe direction.
#[test]
fn an_unrecognised_isolation_value_falls_through() {
    assert_eq!(IsolationTier::parse("wildly-isolated"), None);
    let (tier, reason) = resolve_tier(
        &req(Some("wildly-isolated"), true, true),
        AutoIsolation::Overlap,
        NO_CAP,
    );
    assert_eq!(tier, IsolationTier::Worktree);
    assert_eq!(reason, IsolationReason::OverlappingClaim);
}

/// The property Tier 3 exists for.
#[test]
fn only_the_top_tier_and_the_shared_tree_may_build() {
    assert!(IsolationTier::Shared.may_build());
    assert!(!IsolationTier::Worktree.may_build());
    assert!(IsolationTier::WorktreeWithBuilds.may_build());
}

#[test]
fn only_the_isolated_tiers_get_a_worktree() {
    assert!(!IsolationTier::Shared.needs_worktree());
    assert!(IsolationTier::Worktree.needs_worktree());
    assert!(IsolationTier::WorktreeWithBuilds.needs_worktree());
}

/// The ordering is what `isolation_max_tier` clamps against, so it is asserted
/// rather than assumed.
#[test]
fn tiers_are_ordered_by_cost() {
    assert!(IsolationTier::Shared < IsolationTier::Worktree);
    assert!(IsolationTier::Worktree < IsolationTier::WorktreeWithBuilds);
}

#[test]
fn the_build_refusal_names_both_ways_out() {
    let message = build_refusal("cargo check");
    assert!(message.contains("cargo check"), "{message}");
    assert!(message.contains("after merge"), "{message}");
    assert!(message.contains("worktree-with-builds"), "{message}");
}

// --- build-command classification -----------------------------------------

#[test]
fn the_obvious_build_and_test_commands_are_recognised() {
    for command in [
        "cargo build",
        "cargo test --workspace",
        "cargo check -p archon-core",
        "cargo clippy -- -D warnings",
        "npm run build",
        "pnpm test",
        "yarn lint",
        "make",
        "pytest tests/",
        "go build ./...",
        "mvn package",
    ] {
        assert!(
            build_command_in(command).is_some(),
            "should have been refused: {command}"
        );
    }
}

#[test]
fn ordinary_commands_are_left_alone() {
    for command in [
        "ls -la",
        "git status",
        "rg TODO src/",
        "cat Cargo.toml",
        "git commit -m 'build the thing'",
        "echo cargo build",
    ] {
        assert_eq!(
            build_command_in(command),
            None,
            "should have been allowed: {command}"
        );
    }
}

/// A chained command builds if any part of it does — refusing only when the
/// line *starts* with cargo would be trivially bypassed.
#[test]
fn a_build_hidden_later_in_a_chain_is_still_found() {
    for command in [
        "ls && cargo build",
        "cd /tmp; make",
        "true || pytest",
        "git diff | tee out.txt && npm test",
    ] {
        assert!(
            build_command_in(command).is_some(),
            "should have been refused: {command}"
        );
    }
}

/// The tier cannot be walked around by leaving the worktree, because the gate
/// reads the command rather than the working directory.
#[test]
fn changing_directory_first_does_not_hide_a_build() {
    assert!(build_command_in("cd ../../main-checkout && cargo build").is_some());
}

#[test]
fn env_prefixes_and_wrappers_do_not_hide_the_program() {
    for command in [
        "RUSTFLAGS=-Awarnings cargo build",
        "env CARGO_TERM_COLOR=never cargo test",
        "time cargo check",
        "/usr/bin/cargo build",
    ] {
        assert!(
            build_command_in(command).is_some(),
            "should have been refused: {command}"
        );
    }
}

/// `cargo` alone, or with a non-building subcommand, is not a build.
#[test]
fn non_building_subcommands_are_allowed() {
    for command in ["cargo --version", "cargo metadata", "npm ls", "go version"] {
        assert_eq!(
            build_command_in(command),
            None,
            "should have been allowed: {command}"
        );
    }
}

#[test]
fn the_refusal_names_the_offending_segment() {
    let segment = build_command_in("ls -la && cargo test --workspace").expect("should refuse");
    assert!(segment.contains("cargo test"), "{segment}");
    assert!(!segment.contains("ls -la"), "{segment}");
}

#[test]
fn round_tripping_a_tier_through_its_string_is_stable() {
    for tier in [
        IsolationTier::Shared,
        IsolationTier::Worktree,
        IsolationTier::WorktreeWithBuilds,
    ] {
        assert_eq!(IsolationTier::parse(tier.as_str()), Some(tier));
    }
}
