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
            select_thinking_mode("claude-opus-4-6", 0),
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
}
