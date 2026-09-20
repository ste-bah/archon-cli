//! The compaction trigger must sit below what the server will accept.
//!
//! Servers reserve `max_tokens` out of the context window before the prompt is
//! placed. Compaction used to subtract only `context.output_reserve_tokens`, so
//! it aimed at a window the request was never allowed to fill. These assert the
//! relationship for whatever the config says, not for one deployment's numbers.
use super::types::AgentConfig;

/// Live config as of 2026-09-08.
const WINDOW: u64 = 262_144;
const OUTPUT_RESERVE: u64 = 8_192;
const COMPACT_THRESHOLD: f32 = 0.80;
const SAFETY_MARGIN: f32 = 0.05;

fn config(max_tokens: u32, output_reserve: u64) -> AgentConfig {
    let mut config = AgentConfig::default();
    config.max_tokens = max_tokens;
    config.context.output_reserve_tokens = output_reserve;
    config.context.compact_threshold = COMPACT_THRESHOLD;
    config.context.preflight_safety_margin = SAFETY_MARGIN;
    config
}

/// What `maybe_compact_for_context_window` and its two siblings compute.
fn trigger_tokens(config: &AgentConfig, window: u64) -> f64 {
    let effective = config.effective_context_window(window);
    let threshold =
        (config.context.compact_threshold - config.context.preflight_safety_margin).max(0.0);
    f64::from(threshold) * effective as f64
}

/// The largest prompt the server will accept: the window minus what it holds
/// back for the answer.
fn accepted_prompt_ceiling(config: &AgentConfig, window: u64) -> f64 {
    window.saturating_sub(u64::from(config.max_tokens)) as f64
}

#[test]
fn compaction_triggers_below_the_accepted_prompt_ceiling_at_every_answer_budget() {
    // 8192 sits below output_reserve_tokens, 16384 is the live value, and the
    // rest are budgets an operator could plausibly set on this window.
    for max_tokens in [8_192u32, 16_384, 32_768, 65_536, 131_072] {
        let config = config(max_tokens, OUTPUT_RESERVE);
        let trigger = trigger_tokens(&config, WINDOW);
        let ceiling = accepted_prompt_ceiling(&config, WINDOW);
        assert!(
            trigger < ceiling,
            "max_tokens={max_tokens}: compaction fires at {trigger} but the server \
             rejects anything over {ceiling}"
        );
    }
}

#[test]
fn the_reserve_follows_config_rather_than_a_fixed_number() {
    // Whichever of the two is larger is the real constraint, and both come
    // from config.toml.
    assert_eq!(config(16_384, 8_192).response_reserve_tokens(), 16_384);
    assert_eq!(config(4_096, 8_192).response_reserve_tokens(), 8_192);
    assert_eq!(config(65_536, 8_192).response_reserve_tokens(), 65_536);
    assert_eq!(config(16_384, 40_000).response_reserve_tokens(), 40_000);
}

#[test]
fn the_old_output_reserve_only_maths_is_what_broke() {
    // Reserving only output_reserve_tokens leaves the trigger ABOVE the
    // ceiling at the budget that failed live, which is why six requests were
    // rejected before compaction ever ran. Guards the regression, not the fix.
    let config = config(65_536, OUTPUT_RESERVE);
    let old_effective = WINDOW.saturating_sub(config.context.output_reserve_tokens);
    let old_trigger = f64::from(COMPACT_THRESHOLD - SAFETY_MARGIN) * old_effective as f64;
    let ceiling = accepted_prompt_ceiling(&config, WINDOW);
    assert!(
        old_trigger < ceiling + 8_192.0,
        "sanity: the gap was small, not inverted"
    );
    assert!(
        ceiling - old_trigger < 8_192.0,
        "the old gap was {} tokens; one 24 KB shell result is about 6000",
        ceiling - old_trigger
    );
    assert!(
        trigger_tokens(&config, WINDOW) < old_trigger,
        "the fix must lower the trigger"
    );
}

#[test]
fn an_answer_ceiling_at_or_above_the_window_does_not_switch_compaction_off() {
    // `evaluate_compaction` declines when the window is 0, so a reserve that
    // swallows the window would disable compaction in the one regime where
    // nothing fits. Found by two live tests failing on the first version of
    // this change; kept so the floor cannot be dropped again.
    for (window, max_tokens) in [(100u64, 8_192u32), (8_192, 8_192), (1, 65_536)] {
        let config = config(max_tokens, 0);
        assert!(
            config.effective_context_window(window) > 0,
            "window={window} max_tokens={max_tokens} disabled compaction"
        );
    }
    // A window of zero means "unknown", and stays that way.
    assert_eq!(config(16_384, 8_192).effective_context_window(0), 0);
}
