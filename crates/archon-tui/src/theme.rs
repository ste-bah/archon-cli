//! INTJ color theme for the Archon TUI.
//!
//! Strategic, cold, precise — deep blues, purples, and teals.

use ratatui::style::Color;

/// Complete color palette for the Archon TUI.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Theme {
    /// Primary background.
    pub bg: Color,
    /// Primary foreground text.
    pub fg: Color,
    /// Primary accent (deep teal).
    pub accent: Color,
    /// Secondary accent (deep purple).
    pub accent_secondary: Color,
    /// Header / title color (bright teal).
    pub header: Color,
    /// Muted / disabled text.
    pub muted: Color,
    /// Error indicator.
    pub error: Color,
    /// Success indicator.
    pub success: Color,
    /// Warning indicator.
    pub warning: Color,
    /// Default border color (dim blue).
    pub border: Color,
    /// Active / focused border color (bright blue).
    pub border_active: Color,
    /// Thinking-dot base color.
    pub thinking_dot: Color,
    /// Thinking-dot bright color (animated highlight).
    pub thinking_dot_bright: Color,
}

/// Return the default INTJ theme.
pub fn intj_theme() -> Theme {
    Theme {
        bg: Color::Black,
        fg: Color::Rgb(200, 200, 210),
        accent: Color::Rgb(0, 180, 180),
        accent_secondary: Color::Rgb(130, 80, 200),
        header: Color::Rgb(0, 220, 220),
        muted: Color::DarkGray,
        error: Color::Rgb(200, 60, 60),
        success: Color::Rgb(0, 200, 120),
        warning: Color::Rgb(200, 180, 0),
        border: Color::Rgb(60, 60, 120),
        border_active: Color::Rgb(80, 120, 220),
        thinking_dot: Color::Rgb(0, 180, 180),
        thinking_dot_bright: Color::Rgb(0, 255, 255),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn intj_theme_returns_valid_theme() {
        let theme = intj_theme();
        // Smoke: ensure colors are populated (not all the same).
        assert_ne!(theme.bg, theme.fg);
        assert_ne!(theme.accent, theme.accent_secondary);
    }

    #[test]
    fn all_theme_colors_are_distinct() {
        let t = intj_theme();
        let colors = [
            t.bg,
            t.fg,
            t.accent,
            t.accent_secondary,
            t.header,
            t.muted,
            t.error,
            t.success,
            t.warning,
            t.border,
            t.border_active,
            t.thinking_dot,
            t.thinking_dot_bright,
        ];
        // Every color should be unique except thinking_dot == accent (both teal).
        // Verify at least 11 distinct values out of 13.
        let mut unique = colors.to_vec();
        unique.sort_by(|a, b| format!("{a:?}").cmp(&format!("{b:?}")));
        unique.dedup();
        assert!(
            unique.len() >= 11,
            "expected at least 11 distinct colors, got {}",
            unique.len()
        );
    }

    #[test]
    fn theme_is_clone_and_eq() {
        let a = intj_theme();
        let b = a.clone();
        assert_eq!(a, b);
    }
}
