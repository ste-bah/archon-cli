//! The coordination hooks are still at their choke points (#184 M5, M9).
//!
//! Three one-line calls carry the whole team layer: a roster join where a spawn
//! registers its name, a roster leave where a completion cleans up, and a spawn
//! record for the learning systems. Each is easy to drop in a refactor and
//! nothing else fails when you do — the team simply stops being populated, and
//! `TeamDelete` waits on members that already left. That is the M3 failure this
//! issue already hit once: config that was defined, documented and never read.
//!
//! So this reads the source. Following the precedent of `preserve_d5_agt025.rs`,
//! which guards its invariant the same way and for the same reason: a behaviour
//! whose absence is silent needs a test that fails loudly.

const RUN_PREPARE: &str = include_str!("../src/subagent_executor/run_prepare.rs");
const COMPLETION: &str = include_str!("../src/subagent_executor/completion.rs");

/// Every named spawn funnels through `register_subagent_run`, so the join has to
/// be there and not somewhere a second spawn path could miss.
#[test]
fn a_spawn_seats_its_agent_on_the_team() {
    assert!(
        RUN_PREPARE.contains("team_roster::join("),
        "run_prepare.rs no longer seats spawns on the active team — the roster \
         will stay empty and TeamDelete will wait on members that never joined"
    );
}

/// The leave is also `TeamDelete`'s acknowledgement, so losing it turns every
/// team shutdown into a 60-second wait followed by a refusal.
#[test]
fn a_completion_vacates_its_team_seat() {
    assert!(
        COMPLETION.contains("team_roster::leave("),
        "completion.rs no longer vacates the team seat — TeamDelete's handshake \
         has nothing to wait on"
    );
}

/// Recorded at spawn because write claims are liveness-derived and gone by the
/// time the merge happens. Without it the merge row has an outcome and no
/// context, which is exactly the half that makes it predictive.
#[test]
fn a_spawn_records_what_the_merge_will_need() {
    assert!(
        RUN_PREPARE.contains("coordination_record::record_spawn("),
        "run_prepare.rs no longer records spawn facts — merge outcomes will be \
         recorded with no claim or isolation context to explain them"
    );
}

/// The overlap that drives auto-isolation is the same fact the merge row wants,
/// so it is computed once. Two computations would drift.
#[test]
fn the_overlap_check_feeds_both_isolation_and_the_record() {
    assert!(
        RUN_PREPARE.contains("let claim_overlap ="),
        "run_prepare.rs should compute the claim overlap once and use it for \
         both the isolation decision and the spawn record"
    );
}
