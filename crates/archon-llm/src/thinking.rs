use serde_json::json;

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

const BUDGET_MIN: u32 = 1024;
const BUDGET_MAX: u32 = 131_072;
const THINKING_BETA: &str = "interleaved-thinking-2025-05-14";

// ---------------------------------------------------------------------------
// Thinking mode
// ---------------------------------------------------------------------------

/// Describes how extended thinking should be configured for an API request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ThinkingMode {
    /// Model natively supports adaptive thinking (opus, sonnet).
    Adaptive,
    /// Model uses budget-capped thinking with an explicit token budget.
    Budgeted { budget_tokens: u32 },
    /// Thinking is disabled entirely.
    Disabled,
}

/// Select the appropriate thinking mode for a model + config budget.
///
/// - Models containing "opus" or "sonnet" (case-insensitive, excluding
///   "haiku") get [`ThinkingMode::Adaptive`].
/// - Other models get [`ThinkingMode::Budgeted`] with the budget clamped to
///   `[1024, 131072]`. A zero budget means [`ThinkingMode::Disabled`].
pub fn select_thinking_mode(model: &str, config_budget: u32) -> ThinkingMode {
    if supports_adaptive(model) {
        return ThinkingMode::Adaptive;
    }

    if config_budget == 0 {
        return ThinkingMode::Disabled;
    }

    ThinkingMode::Budgeted {
        budget_tokens: config_budget.clamp(BUDGET_MIN, BUDGET_MAX),
    }
}

/// Whether this turn's input asks for maximum reasoning via the `ultrathink`
/// keyword.
///
/// Case-insensitive substring match, matching the TUI's own highlight scan so
/// the rainbow-coloured word and the escalated request never disagree.
pub fn ultrathink_requested(user_input: &str) -> bool {
    user_input.to_lowercase().contains("ultrathink")
}

/// Escalate a thinking mode for an `ultrathink` turn (#123).
///
/// - [`ThinkingMode::Budgeted`] is raised to the largest budget that still
///   leaves room under `max_tokens`. Anthropic requires `budget_tokens` to be
///   strictly less than `max_tokens`, so the ceiling is clamped rather than
///   blindly set to [`BUDGET_MAX`].
/// - [`ThinkingMode::Adaptive`] is returned unchanged. Adaptive thinking
///   exposes no depth knob — there is no budget to raise — so on Opus/Sonnet
///   the only available lever is effort, which `turn_effort` pushes to `Max`
///   for the same turn. Inventing a budget here would silently drop the model
///   OUT of adaptive mode, which is a downgrade, not an escalation.
/// - [`ThinkingMode::Disabled`] is returned unchanged. That state means the
///   operator set `thinking_budget = 0`, or the model has no thinking at all;
///   a keyword in a prompt should not override either.
pub fn escalated_for_ultrathink(mode: ThinkingMode, max_tokens: u32) -> ThinkingMode {
    match mode {
        ThinkingMode::Budgeted { budget_tokens } => {
            let ceiling = max_tokens.saturating_sub(1).min(BUDGET_MAX);
            // `max`, not a plain assignment: if the configured budget is
            // already above the ceiling this leaves it alone rather than
            // silently shrinking it. A configured budget that exceeds
            // `max_tokens` is a pre-existing config problem and not this
            // function's to rewrite — escalation must never lower a budget.
            let raised = budget_tokens.max(ceiling);
            ThinkingMode::Budgeted {
                budget_tokens: raised.clamp(BUDGET_MIN, BUDGET_MAX),
            }
        }
        other => other,
    }
}

/// Build the JSON `thinking` parameter for an API request body.
pub fn thinking_param(mode: &ThinkingMode) -> Option<serde_json::Value> {
    match mode {
        ThinkingMode::Adaptive => Some(json!({ "type": "adaptive" })),
        ThinkingMode::Budgeted { budget_tokens } => Some(json!({
            "type": "enabled",
            "budget_tokens": budget_tokens,
        })),
        ThinkingMode::Disabled => None,
    }
}

/// Return the beta header strings required for thinking.
pub fn thinking_betas(mode: &ThinkingMode) -> Vec<String> {
    match mode {
        ThinkingMode::Disabled => Vec::new(),
        _ => vec![THINKING_BETA.to_owned()],
    }
}

// ---------------------------------------------------------------------------
// TUI display state
// ---------------------------------------------------------------------------

/// Accumulated thinking display state for the TUI.
#[derive(Debug, Clone, Default)]
pub struct ThinkingDisplay {
    /// Whether the thinking panel is visible (toggled by `/thinking`).
    pub visible: bool,
    /// Accumulated thinking text from streaming events.
    pub current_thinking_text: String,
    /// Total thinking tokens consumed so far.
    pub thinking_tokens: u32,
    /// Wall-clock thinking duration in milliseconds.
    pub thinking_duration_ms: u64,
}

impl ThinkingDisplay {
    /// Reset accumulated thinking state for a new turn.
    pub fn reset(&mut self) {
        self.current_thinking_text.clear();
        self.thinking_tokens = 0;
        self.thinking_duration_ms = 0;
    }

    /// Toggle visibility and return the new state.
    pub fn toggle_visible(&mut self) -> bool {
        self.visible = !self.visible;
        self.visible
    }
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

fn supports_adaptive(model: &str) -> bool {
    let lower = model.to_lowercase();
    (lower.contains("opus") || lower.contains("sonnet")) && !lower.contains("haiku")
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // -- select_thinking_mode -------------------------------------------------

    #[test]
    fn adaptive_for_opus() {
        assert_eq!(
            select_thinking_mode("claude-opus-4-8", 0),
            ThinkingMode::Adaptive,
        );
    }

    #[test]
    fn adaptive_for_sonnet() {
        assert_eq!(
            select_thinking_mode("claude-sonnet-4-6", 8192),
            ThinkingMode::Adaptive,
        );
    }

    #[test]
    fn adaptive_case_insensitive() {
        assert_eq!(
            select_thinking_mode("Claude-OPUS-4-6", 0),
            ThinkingMode::Adaptive,
        );
    }

    #[test]
    fn haiku_not_adaptive() {
        // haiku contains neither opus nor sonnet in practice, but guard
        // the explicit exclusion anyway.
        assert_ne!(
            select_thinking_mode("claude-haiku-4-5", 8192),
            ThinkingMode::Adaptive,
        );
    }

    #[test]
    fn budgeted_for_unknown_model() {
        assert_eq!(
            select_thinking_mode("gpt-4o", 16384),
            ThinkingMode::Budgeted {
                budget_tokens: 16384
            },
        );
    }

    #[test]
    fn disabled_when_zero_budget_non_adaptive() {
        assert_eq!(select_thinking_mode("gpt-4o", 0), ThinkingMode::Disabled,);
    }

    // -- budget clamping ------------------------------------------------------

    #[test]
    fn budget_clamped_low() {
        assert_eq!(
            select_thinking_mode("unknown-model", 100),
            ThinkingMode::Budgeted {
                budget_tokens: BUDGET_MIN
            },
        );
    }

    #[test]
    fn budget_clamped_high() {
        assert_eq!(
            select_thinking_mode("unknown-model", 999_999),
            ThinkingMode::Budgeted {
                budget_tokens: BUDGET_MAX
            },
        );
    }

    // -- thinking_param -------------------------------------------------------

    #[test]
    fn param_adaptive_json() {
        let val = thinking_param(&ThinkingMode::Adaptive).expect("should be Some");
        assert_eq!(val["type"], "adaptive");
    }

    #[test]
    fn param_budgeted_json() {
        let mode = ThinkingMode::Budgeted {
            budget_tokens: 8192,
        };
        let val = thinking_param(&mode).expect("should be Some");
        assert_eq!(val["type"], "enabled");
        assert_eq!(val["budget_tokens"], 8192);
    }

    #[test]
    fn param_disabled_none() {
        assert!(thinking_param(&ThinkingMode::Disabled).is_none());
    }

    // -- thinking_betas -------------------------------------------------------

    #[test]
    fn betas_present_when_adaptive() {
        let betas = thinking_betas(&ThinkingMode::Adaptive);
        assert_eq!(betas.len(), 1);
        assert_eq!(betas[0], THINKING_BETA);
    }

    #[test]
    fn betas_present_when_budgeted() {
        let mode = ThinkingMode::Budgeted {
            budget_tokens: 4096,
        };
        let betas = thinking_betas(&mode);
        assert_eq!(betas.len(), 1);
    }

    #[test]
    fn betas_empty_when_disabled() {
        assert!(thinking_betas(&ThinkingMode::Disabled).is_empty());
    }

    // -- ThinkingDisplay ------------------------------------------------------

    #[test]
    fn display_default_state() {
        let d = ThinkingDisplay::default();
        assert!(!d.visible);
        assert!(d.current_thinking_text.is_empty());
        assert_eq!(d.thinking_tokens, 0);
        assert_eq!(d.thinking_duration_ms, 0);
    }

    #[test]
    fn display_toggle() {
        let mut d = ThinkingDisplay::default();
        assert!(d.toggle_visible());
        assert!(!d.toggle_visible());
    }

    #[test]
    fn display_reset() {
        let mut d = ThinkingDisplay {
            visible: true,
            current_thinking_text: "thinking...".into(),
            thinking_tokens: 500,
            thinking_duration_ms: 1234,
        };
        d.reset();
        assert!(d.visible, "reset should not change visibility");
        assert!(d.current_thinking_text.is_empty());
        assert_eq!(d.thinking_tokens, 0);
        assert_eq!(d.thinking_duration_ms, 0);
    }

    // -- ultrathink escalation (#123) -----------------------------------------

    #[test]
    fn ultrathink_keyword_detection_is_case_insensitive_and_substring() {
        assert!(ultrathink_requested("please ULTRATHINK this"));
        assert!(ultrathink_requested("ultrathink"));
        assert!(ultrathink_requested("go ultrathinking"));
        assert!(!ultrathink_requested("think hard about it"));
        assert!(!ultrathink_requested(""));
    }

    #[test]
    fn ultrathink_raises_a_budgeted_mode() {
        let raised = escalated_for_ultrathink(
            ThinkingMode::Budgeted {
                budget_tokens: 4096,
            },
            65_536,
        );
        assert_eq!(
            raised,
            ThinkingMode::Budgeted {
                budget_tokens: 65_535
            }
        );
    }

    #[test]
    fn ultrathink_budget_stays_under_max_tokens() {
        let raised = escalated_for_ultrathink(
            ThinkingMode::Budgeted {
                budget_tokens: 2048,
            },
            8192,
        );
        let ThinkingMode::Budgeted { budget_tokens } = raised else {
            panic!("expected a budgeted mode, got {raised:?}");
        };
        assert!(
            budget_tokens < 8192,
            "budget must leave room under max_tokens, got {budget_tokens}"
        );
    }

    #[test]
    fn ultrathink_never_lowers_an_already_larger_budget() {
        // A configured budget above the ceiling is a pre-existing config
        // problem; escalation must not quietly shrink it.
        let raised = escalated_for_ultrathink(
            ThinkingMode::Budgeted {
                budget_tokens: 16_384,
            },
            8192,
        );
        assert_eq!(
            raised,
            ThinkingMode::Budgeted {
                budget_tokens: 16_384
            }
        );
    }

    /// Adaptive thinking has no depth knob — there is no budget to raise, and
    /// swapping it for an explicit budget would drop the model OUT of adaptive
    /// mode. On Opus/Sonnet the escalation is carried by effort instead.
    #[test]
    fn ultrathink_leaves_adaptive_alone() {
        assert_eq!(
            escalated_for_ultrathink(ThinkingMode::Adaptive, 65_536),
            ThinkingMode::Adaptive
        );
    }

    /// `Disabled` means the operator set `thinking_budget = 0`, or the model
    /// has no thinking at all. A keyword in a prompt should not override that.
    #[test]
    fn ultrathink_does_not_resurrect_disabled_thinking() {
        assert_eq!(
            escalated_for_ultrathink(ThinkingMode::Disabled, 65_536),
            ThinkingMode::Disabled
        );
    }
}
