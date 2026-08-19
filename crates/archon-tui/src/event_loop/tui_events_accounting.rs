//! Token and cost accounting for the two events that carry usage numbers.
//!
//! Split out of `event_loop/tui_events.rs` when that file reached the 500-line
//! ceiling, the same reason `picker_input.rs` and `picker_events.rs` were split
//! out of their parents. These two arms were the fattest in the match and the
//! only ones doing arithmetic rather than assignment, so they are also the pair
//! that most benefits from being readable on their own.

use crate::app::App;

/// Fold one turn's usage into the status bar.
///
/// Context is assigned, not accumulated: it is the pressure of the current
/// turn, not a session total. Full context is billable input plus both cache
/// figures, which is the same value the compaction trigger reads, so the number
/// on screen and the number that decides when to compact cannot disagree.
///
/// Cache totals *are* accumulated, because those are cost, and cost is
/// cumulative.
pub(crate) fn apply_turn_usage(
    app: &mut App,
    input_tokens: u64,
    output_tokens: u64,
    cache_creation_tokens: u64,
    cache_read_tokens: u64,
) {
    app.on_turn_complete();
    app.status.cost += archon_core::cost::estimate_turn_cost_usd(
        &app.status.model,
        input_tokens,
        output_tokens,
        cache_creation_tokens,
        cache_read_tokens,
    );

    let context_tokens = input_tokens
        .saturating_add(cache_creation_tokens)
        .saturating_add(cache_read_tokens);
    // Zero means the provider reported no usage for this turn, which is not the
    // same as a turn that used no context. Leaving the previous figure standing
    // is more honest than resetting the bar to nothing.
    if context_tokens > 0 {
        app.status.context_tokens_used = context_tokens;
    }

    app.status.cache_creation_tokens = app
        .status
        .cache_creation_tokens
        .saturating_add(cache_creation_tokens);
    app.status.cache_read_tokens = app
        .status
        .cache_read_tokens
        .saturating_add(cache_read_tokens);
    app.status.update_context_warning();
}

/// Replace the status bar's context figures wholesale.
///
/// Unlike a turn's usage this is a measurement of the whole conversation taken
/// elsewhere, so every field is assigned rather than added.
#[allow(clippy::too_many_arguments)]
pub(crate) fn apply_context_pressure(
    app: &mut App,
    tokens_used: u64,
    context_window: u64,
    cache_creation_tokens: u64,
    cache_read_tokens: u64,
    context_name: Option<String>,
    resolution_source: Option<String>,
    heaviest_message_tokens: u64,
) {
    app.status.heaviest_message_tokens = heaviest_message_tokens;
    app.status.context_tokens_used = tokens_used;
    app.status.context_window = context_window;
    app.status.cache_creation_tokens = cache_creation_tokens;
    app.status.cache_read_tokens = cache_read_tokens;
    app.status.context_name = context_name;
    app.status.resolution_source = resolution_source;
    app.status.update_context_warning();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_turn_assigns_context_and_accumulates_cache() {
        let mut app = App::default();
        apply_turn_usage(&mut app, 100, 20, 30, 40);
        assert_eq!(app.status.context_tokens_used, 170);
        assert_eq!(app.status.cache_creation_tokens, 30);

        apply_turn_usage(&mut app, 10, 2, 3, 4);
        assert_eq!(
            app.status.context_tokens_used, 17,
            "context is this turn's pressure, not a running total"
        );
        assert_eq!(
            app.status.cache_creation_tokens, 33,
            "cache is cost, and cost accumulates"
        );
    }

    /// A turn the provider reported no usage for must not blank the bar.
    #[test]
    fn a_zero_usage_turn_leaves_the_previous_context_standing() {
        let mut app = App::default();
        apply_turn_usage(&mut app, 100, 20, 0, 0);
        apply_turn_usage(&mut app, 0, 5, 0, 0);
        assert_eq!(app.status.context_tokens_used, 100);
    }

    #[test]
    fn context_pressure_replaces_rather_than_accumulates() {
        let mut app = App::default();
        apply_turn_usage(&mut app, 100, 20, 30, 40);
        apply_context_pressure(
            &mut app,
            500,
            1000,
            5,
            6,
            Some("main".into()),
            Some("config".into()),
            7,
        );

        assert_eq!(app.status.context_tokens_used, 500);
        assert_eq!(app.status.cache_creation_tokens, 5, "assigned, not added");
        assert_eq!(app.status.heaviest_message_tokens, 7);
        assert_eq!(app.status.context_window, 1000);
    }
}
