// When the admission ledger has anything to say.
//
// Split from `04_tests_policy.rs` to hold the 500-line gate; these are about
// one predicate rather than about policy parsing generally. `include!`d rather
// than a module, so the header is a plain comment: an inner doc comment is only
// legal at the top of a real module file.

/// The ledger half must decline exactly when admission declines.
///
/// Otherwise a guardrail nobody enabled still costs a decision-store read and
/// an `unavailable` row on every non-`Safe` tool call — and a workflow script
/// may make five hundred of them in a run.
#[test]
fn the_tool_run_ledger_is_inactive_when_the_guardrail_is_off() {
    let mut config = archon_core::config::ArchonConfig::default();
    config.learning.world_model.guardrails.enabled = false;
    config.learning.world_model.guardrails.tool_run_mode = "guarded".into();

    assert!(
        !tool_run_ledger_active(&config),
        "a disabled guardrail must not record outcomes"
    );
}

#[test]
fn the_tool_run_ledger_is_inactive_when_the_surface_mode_is_off() {
    let mut config = archon_core::config::ArchonConfig::default();
    config.learning.world_model.guardrails.enabled = true;
    config.learning.world_model.guardrails.tool_run_mode = "off".into();

    assert!(
        !tool_run_ledger_active(&config),
        "a ToolRun surface set to off must not record outcomes"
    );
}

/// ...and it must still be active when the guardrail IS on, or the ledger would
/// be silently disabled everywhere.
#[test]
fn the_tool_run_ledger_is_active_when_the_guardrail_is_on() {
    let mut config = archon_core::config::ArchonConfig::default();
    config.learning.world_model.guardrails.enabled = true;
    config.learning.world_model.guardrails.tool_run_mode = "guarded".into();

    assert!(tool_run_ledger_active(&config));
}
