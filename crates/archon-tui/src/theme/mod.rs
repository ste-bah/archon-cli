//! Theme system for the Archon TUI.
//!
//! Relocated from `src/theme.rs` → `src/theme/` per REM-2i (REM-2 split plan,
//! docs/rem-2-split-plan.md section 5). Zero public-API change: every
//! `archon_tui::theme::*` path is preserved via flat re-exports below.
//!
//! Submodules:
//! - `mbti` — 16 MBTI-typed theme constructors (intj, intp, …, esfp).
//! - `classic` — 7 utility theme constructors (dark, light, ocean, fire,
//!   forest, daltonized, mono).

use ratatui::style::Color;

mod classic;
mod mbti;

pub use classic::{
    daltonized_theme, dark_theme, fire_theme, forest_theme, light_theme, mono_theme, ocean_theme,
};
pub use mbti::{
    enfj_theme, enfp_theme, entj_theme, entp_theme, esfj_theme, esfp_theme, estj_theme, estp_theme,
    infj_theme, infp_theme, intj_theme, intp_theme, isfj_theme, isfp_theme, istj_theme, istp_theme,
};

// ---------------------------------------------------------------------------
// Named color parser
// ---------------------------------------------------------------------------

/// Parse a color name string into a ratatui `Color`.
///
/// Accepts: red, green, yellow, blue, magenta, cyan, white, default/reset.
/// Returns `None` for unknown names.
pub fn parse_color(name: &str) -> Option<Color> {
    match name.to_lowercase().as_str() {
        "red" => Some(Color::Red),
        "green" => Some(Color::Green),
        "yellow" => Some(Color::Yellow),
        "blue" => Some(Color::Blue),
        "magenta" | "purple" => Some(Color::Magenta),
        "cyan" => Some(Color::Cyan),
        "white" => Some(Color::White),
        "default" | "reset" | "none" => Some(Color::Rgb(0, 180, 180)), // revert to INTJ teal
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// Named theme presets
// ---------------------------------------------------------------------------

/// Parse a theme name and return a full `Theme`.
///
/// Accepts all 16 MBTI types plus utility themes: dark, light, ocean, fire,
/// forest, mono, daltonized.  `"auto"` is handled by `ThemeRegistry::resolve`.
/// Returns `None` for unknown names.
pub fn theme_by_name(name: &str) -> Option<Theme> {
    match name.to_lowercase().as_str() {
        // MBTI themes
        "intj" | "default" => Some(intj_theme()),
        "intp" => Some(intp_theme()),
        "entj" => Some(entj_theme()),
        "entp" => Some(entp_theme()),
        "infj" => Some(infj_theme()),
        "infp" => Some(infp_theme()),
        "enfj" => Some(enfj_theme()),
        "enfp" => Some(enfp_theme()),
        "istj" => Some(istj_theme()),
        "isfj" => Some(isfj_theme()),
        "estj" => Some(estj_theme()),
        "esfj" => Some(esfj_theme()),
        "istp" => Some(istp_theme()),
        "isfp" => Some(isfp_theme()),
        "estp" => Some(estp_theme()),
        "esfp" => Some(esfp_theme()),
        // Utility themes
        "dark" => Some(dark_theme()),
        "light" => Some(light_theme()),
        "ocean" => Some(ocean_theme()),
        "fire" => Some(fire_theme()),
        "forest" => Some(forest_theme()),
        "mono" => Some(mono_theme()),
        "daltonized" => Some(daltonized_theme()),
        _ => None,
    }
}

/// List all 22 built-in named themes (excludes `auto` which is dynamic).
pub fn available_themes() -> &'static [&'static str] {
    &[
        // MBTI
        "intj", "intp", "entj", "entp", "infj", "infp", "enfj", "enfp", "istj", "isfj", "estj",
        "esfj", "istp", "isfp", "estp", "esfp", // Utility
        "dark", "light", "ocean", "fire", "forest", "mono",
    ]
}

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

impl Theme {
    /// Parse a `"#RRGGBB"` or `"RRGGBB"` hex string into a `ratatui::Color::Rgb`.
    ///
    /// Returns `None` for invalid input (wrong length, non-hex characters).
    pub fn from_hex(hex: &str) -> Option<Color> {
        let s = hex.trim_start_matches('#');
        if s.len() != 6 {
            return None;
        }
        let r = u8::from_str_radix(&s[0..2], 16).ok()?;
        let g = u8::from_str_radix(&s[2..4], 16).ok()?;
        let b = u8::from_str_radix(&s[4..6], 16).ok()?;
        Some(Color::Rgb(r, g, b))
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
