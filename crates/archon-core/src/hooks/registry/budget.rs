//! Which deadline governs a single hook run.
//!
//! Two clocks bound one hook: the timeout it declares for itself, and whatever
//! is left of the event's aggregate budget. The shorter one has to win, or a
//! single 60-second hook late in a 30-second event would outlive the budget it
//! was supposed to be spending.

use std::time::Duration;

use crate::hooks::types::HookConfig;

/// Timeout applied to a hook that declares none.
pub(crate) const DEFAULT_HOOK_TIMEOUT_SECS: u32 = 60;

/// The timeout a hook actually runs under: its own, or what remains of the
/// aggregate budget, whichever is shorter.
///
/// [`HookConfig::timeout`] has second granularity, so a sub-second remainder
/// cannot be expressed and floors at one second rather than truncating to zero.
/// A hook with no budget left at all never reaches here — the caller's
/// budget-exhausted branch skips it and applies its failure policy instead — so
/// the floor only ever rounds *up* a hook that was going to run anyway.
pub(crate) fn effective_hook_timeout_secs(
    hook_timeout_secs: Option<u32>,
    remaining: Duration,
) -> u32 {
    let remaining_secs = remaining.as_secs().max(1) as u32;
    hook_timeout_secs
        .unwrap_or(DEFAULT_HOOK_TIMEOUT_SECS)
        .min(remaining_secs)
}

/// `hook` as it should be handed to the executor under `remaining` budget.
///
/// The returned config always carries an explicit timeout, including for a hook
/// that declared none: the executor's own `unwrap_or(60)` would otherwise
/// re-apply the unclamped default and undo the clamp.
pub(crate) fn clamp_hook_to_budget(hook: &HookConfig, remaining: Duration) -> HookConfig {
    let mut clamped = hook.clone();
    clamped.timeout = Some(effective_hook_timeout_secs(hook.timeout, remaining));
    clamped
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The whole point of the clamp, stated without a clock: a hook that asks
    /// for longer than the budget has left gets the budget's answer, not its
    /// own. Asserting this on elapsed wall time instead measured how loaded the
    /// machine was, and failed on a busy one while the clamp worked perfectly.
    #[test]
    fn remaining_budget_wins_when_it_is_shorter_than_the_hook_timeout() {
        assert_eq!(
            effective_hook_timeout_secs(Some(60), Duration::from_secs(2)),
            2
        );
    }

    #[test]
    fn hook_timeout_wins_when_it_is_shorter_than_the_remaining_budget() {
        assert_eq!(
            effective_hook_timeout_secs(Some(5), Duration::from_secs(30)),
            5
        );
    }

    #[test]
    fn an_undeclared_timeout_is_still_clamped_to_the_budget() {
        assert_eq!(
            effective_hook_timeout_secs(None, Duration::from_secs(3)),
            3,
            "a hook that declares no timeout must not escape the aggregate budget"
        );
    }

    #[test]
    fn an_undeclared_timeout_falls_back_to_the_default_under_a_wide_budget() {
        assert_eq!(
            effective_hook_timeout_secs(None, Duration::from_secs(600)),
            DEFAULT_HOOK_TIMEOUT_SECS
        );
    }

    #[test]
    fn a_sub_second_remainder_floors_at_one_second_rather_than_zero() {
        assert_eq!(
            effective_hook_timeout_secs(Some(60), Duration::from_millis(1)),
            1,
            "a zero timeout would kill the hook before it could do anything"
        );
    }

    #[test]
    fn the_clamped_config_carries_the_budget_deadline_explicitly() {
        let hook = HookConfig {
            hook_type: crate::hooks::types::HookCommandType::Command,
            command: "sleep 5".to_string(),
            if_condition: None,
            timeout: None,
            once: None,
            r#async: None,
            async_rewake: None,
            status_message: None,
            headers: std::collections::HashMap::new(),
            allowed_env_vars: Vec::new(),
            on_failure: None,
            enabled: true,
        };

        let clamped = clamp_hook_to_budget(&hook, Duration::from_secs(2));

        assert_eq!(
            clamped.timeout,
            Some(2),
            "the executor reads config.timeout; leaving it None re-applies the 60s default"
        );
        assert_eq!(clamped.command, hook.command);
    }
}
