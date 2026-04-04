//! INTJ color theme for the Archon TUI.
//!
//! Strategic, cold, precise — deep blues, purples, and teals.

use ratatui::style::Color;

// ---------------------------------------------------------------------------
// Named color parser
// ---------------------------------------------------------------------------

/// Parse a color name string into a ratatui `Color`.
///
/// Accepts: red, green, yellow, blue, magenta, cyan, white, default/reset.
/// Returns `None` for unknown names.
pub fn parse_color(name: &str) -> Option<Color> {
    match name.to_lowercase().as_str() {
        "red"     => Some(Color::Red),
        "green"   => Some(Color::Green),
        "yellow"  => Some(Color::Yellow),
        "blue"    => Some(Color::Blue),
        "magenta" | "purple" => Some(Color::Magenta),
        "cyan"    => Some(Color::Cyan),
        "white"   => Some(Color::White),
        "default" | "reset" | "none" => Some(Color::Rgb(0, 180, 180)), // revert to INTJ teal
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// Named theme presets
// ---------------------------------------------------------------------------

/// Parse a theme name and return a full `Theme`.
///
/// Accepts: intj (default), dark, light, ocean, fire, forest, mono.
/// Returns `None` for unknown names.
pub fn theme_by_name(name: &str) -> Option<Theme> {
    match name.to_lowercase().as_str() {
        "intj" | "default" => Some(intj_theme()),
        "dark"   => Some(dark_theme()),
        "light"  => Some(light_theme()),
        "ocean"  => Some(ocean_theme()),
        "fire"   => Some(fire_theme()),
        "forest" => Some(forest_theme()),
        "mono"   => Some(mono_theme()),
        _ => None,
    }
}

/// List all available theme names.
pub fn available_themes() -> &'static [&'static str] {
    &["intj", "dark", "light", "ocean", "fire", "forest", "mono"]
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

/// High-contrast dark theme (ANSI colors only).
pub fn dark_theme() -> Theme {
    Theme {
        bg: Color::Black,
        fg: Color::White,
        accent: Color::Cyan,
        accent_secondary: Color::Magenta,
        header: Color::Cyan,
        muted: Color::DarkGray,
        error: Color::Red,
        success: Color::Green,
        warning: Color::Yellow,
        border: Color::DarkGray,
        border_active: Color::Cyan,
        thinking_dot: Color::Cyan,
        thinking_dot_bright: Color::White,
    }
}

/// Light terminal theme (dark text on light background).
pub fn light_theme() -> Theme {
    Theme {
        bg: Color::White,
        fg: Color::Black,
        accent: Color::Blue,
        accent_secondary: Color::Magenta,
        header: Color::Blue,
        muted: Color::DarkGray,
        error: Color::Red,
        success: Color::Green,
        warning: Color::Yellow,
        border: Color::DarkGray,
        border_active: Color::Blue,
        thinking_dot: Color::Blue,
        thinking_dot_bright: Color::Cyan,
    }
}

/// Ocean theme — deep blues and aquas.
pub fn ocean_theme() -> Theme {
    Theme {
        bg: Color::Rgb(5, 20, 40),
        fg: Color::Rgb(180, 210, 240),
        accent: Color::Rgb(0, 150, 220),
        accent_secondary: Color::Rgb(0, 200, 200),
        header: Color::Rgb(100, 200, 255),
        muted: Color::Rgb(80, 100, 130),
        error: Color::Rgb(220, 80, 80),
        success: Color::Rgb(0, 200, 140),
        warning: Color::Rgb(220, 180, 0),
        border: Color::Rgb(30, 60, 100),
        border_active: Color::Rgb(0, 120, 200),
        thinking_dot: Color::Rgb(0, 150, 220),
        thinking_dot_bright: Color::Rgb(100, 220, 255),
    }
}

/// Fire theme — reds, oranges, ambers.
pub fn fire_theme() -> Theme {
    Theme {
        bg: Color::Rgb(20, 5, 5),
        fg: Color::Rgb(240, 210, 180),
        accent: Color::Rgb(220, 80, 20),
        accent_secondary: Color::Rgb(200, 160, 0),
        header: Color::Rgb(255, 120, 40),
        muted: Color::Rgb(120, 80, 60),
        error: Color::Rgb(255, 60, 60),
        success: Color::Rgb(80, 200, 80),
        warning: Color::Rgb(255, 200, 0),
        border: Color::Rgb(100, 40, 20),
        border_active: Color::Rgb(220, 80, 20),
        thinking_dot: Color::Rgb(220, 80, 20),
        thinking_dot_bright: Color::Rgb(255, 160, 60),
    }
}

/// Forest theme — greens and earthy tones.
pub fn forest_theme() -> Theme {
    Theme {
        bg: Color::Rgb(5, 20, 10),
        fg: Color::Rgb(190, 220, 190),
        accent: Color::Rgb(40, 180, 80),
        accent_secondary: Color::Rgb(120, 160, 40),
        header: Color::Rgb(80, 220, 100),
        muted: Color::Rgb(80, 110, 80),
        error: Color::Rgb(220, 80, 60),
        success: Color::Rgb(60, 220, 100),
        warning: Color::Rgb(200, 180, 40),
        border: Color::Rgb(30, 80, 40),
        border_active: Color::Rgb(40, 160, 80),
        thinking_dot: Color::Rgb(40, 180, 80),
        thinking_dot_bright: Color::Rgb(100, 255, 120),
    }
}

/// Monochrome theme — white on black, no color.
pub fn mono_theme() -> Theme {
    Theme {
        bg: Color::Black,
        fg: Color::White,
        accent: Color::White,
        accent_secondary: Color::DarkGray,
        header: Color::White,
        muted: Color::DarkGray,
        error: Color::White,
        success: Color::White,
        warning: Color::White,
        border: Color::DarkGray,
        border_active: Color::White,
        thinking_dot: Color::DarkGray,
        thinking_dot_bright: Color::White,
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
