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
/// Accepts all 16 MBTI types plus utility themes: dark, light, ocean, fire,
/// forest, mono, daltonized.  `"auto"` is handled by `ThemeRegistry::resolve`.
/// Returns `None` for unknown names.
pub fn theme_by_name(name: &str) -> Option<Theme> {
    match name.to_lowercase().as_str() {
        // MBTI themes
        "intj" | "default" => Some(intj_theme()),
        "intp"  => Some(intp_theme()),
        "entj"  => Some(entj_theme()),
        "entp"  => Some(entp_theme()),
        "infj"  => Some(infj_theme()),
        "infp"  => Some(infp_theme()),
        "enfj"  => Some(enfj_theme()),
        "enfp"  => Some(enfp_theme()),
        "istj"  => Some(istj_theme()),
        "isfj"  => Some(isfj_theme()),
        "estj"  => Some(estj_theme()),
        "esfj"  => Some(esfj_theme()),
        "istp"  => Some(istp_theme()),
        "isfp"  => Some(isfp_theme()),
        "estp"  => Some(estp_theme()),
        "esfp"  => Some(esfp_theme()),
        // Utility themes
        "dark"       => Some(dark_theme()),
        "light"      => Some(light_theme()),
        "ocean"      => Some(ocean_theme()),
        "fire"       => Some(fire_theme()),
        "forest"     => Some(forest_theme()),
        "mono"       => Some(mono_theme()),
        "daltonized" => Some(daltonized_theme()),
        _ => None,
    }
}

/// List all 22 built-in named themes (excludes `auto` which is dynamic).
pub fn available_themes() -> &'static [&'static str] {
    &[
        // MBTI
        "intj", "intp", "entj", "entp",
        "infj", "infp", "enfj", "enfp",
        "istj", "isfj", "estj", "esfj",
        "istp", "isfp", "estp", "esfp",
        // Utility
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

// ---------------------------------------------------------------------------
// MBTI themes
// ---------------------------------------------------------------------------

/// INTP — The Logician. Cold blue-grey analytical precision.
pub fn intp_theme() -> Theme {
    Theme {
        bg: Color::Rgb(8, 12, 20),
        fg: Color::Rgb(180, 185, 205),
        accent: Color::Rgb(80, 130, 200),
        accent_secondary: Color::Rgb(100, 100, 160),
        header: Color::Rgb(130, 175, 245),
        muted: Color::Rgb(70, 80, 100),
        error: Color::Rgb(200, 60, 60),
        success: Color::Rgb(60, 200, 120),
        warning: Color::Rgb(200, 180, 40),
        border: Color::Rgb(35, 45, 75),
        border_active: Color::Rgb(80, 130, 200),
        thinking_dot: Color::Rgb(80, 130, 200),
        thinking_dot_bright: Color::Rgb(160, 205, 255),
    }
}

/// ENTJ — The Commander. Imperial deep blue and gold.
pub fn entj_theme() -> Theme {
    Theme {
        bg: Color::Rgb(8, 8, 18),
        fg: Color::Rgb(225, 215, 185),
        accent: Color::Rgb(200, 160, 20),
        accent_secondary: Color::Rgb(40, 80, 180),
        header: Color::Rgb(245, 200, 40),
        muted: Color::Rgb(100, 90, 60),
        error: Color::Rgb(220, 60, 40),
        success: Color::Rgb(60, 200, 100),
        warning: Color::Rgb(240, 160, 20),
        border: Color::Rgb(55, 45, 15),
        border_active: Color::Rgb(200, 160, 20),
        thinking_dot: Color::Rgb(200, 160, 20),
        thinking_dot_bright: Color::Rgb(255, 220, 80),
    }
}

/// ENTP — The Debater. Electric cyan-green, innovative spark.
pub fn entp_theme() -> Theme {
    Theme {
        bg: Color::Rgb(5, 15, 15),
        fg: Color::Rgb(180, 240, 205),
        accent: Color::Rgb(0, 220, 150),
        accent_secondary: Color::Rgb(80, 200, 255),
        header: Color::Rgb(100, 255, 185),
        muted: Color::Rgb(60, 100, 80),
        error: Color::Rgb(220, 80, 60),
        success: Color::Rgb(0, 220, 150),
        warning: Color::Rgb(200, 200, 40),
        border: Color::Rgb(15, 70, 55),
        border_active: Color::Rgb(0, 200, 150),
        thinking_dot: Color::Rgb(0, 220, 150),
        thinking_dot_bright: Color::Rgb(100, 255, 205),
    }
}

/// INFJ — The Advocate. Deep violet and indigo, mystical.
pub fn infj_theme() -> Theme {
    Theme {
        bg: Color::Rgb(10, 5, 20),
        fg: Color::Rgb(210, 190, 235),
        accent: Color::Rgb(140, 80, 220),
        accent_secondary: Color::Rgb(80, 120, 200),
        header: Color::Rgb(185, 125, 255),
        muted: Color::Rgb(80, 60, 100),
        error: Color::Rgb(220, 60, 80),
        success: Color::Rgb(60, 200, 130),
        warning: Color::Rgb(200, 160, 40),
        border: Color::Rgb(50, 28, 80),
        border_active: Color::Rgb(140, 80, 220),
        thinking_dot: Color::Rgb(140, 80, 220),
        thinking_dot_bright: Color::Rgb(205, 155, 255),
    }
}

/// INFP — The Mediator. Soft rose and warm lavender, dreamy.
pub fn infp_theme() -> Theme {
    Theme {
        bg: Color::Rgb(15, 8, 15),
        fg: Color::Rgb(235, 215, 225),
        accent: Color::Rgb(200, 100, 160),
        accent_secondary: Color::Rgb(160, 100, 200),
        header: Color::Rgb(245, 145, 195),
        muted: Color::Rgb(100, 70, 90),
        error: Color::Rgb(220, 60, 80),
        success: Color::Rgb(80, 200, 130),
        warning: Color::Rgb(220, 170, 60),
        border: Color::Rgb(80, 38, 58),
        border_active: Color::Rgb(200, 100, 160),
        thinking_dot: Color::Rgb(200, 100, 160),
        thinking_dot_bright: Color::Rgb(255, 165, 205),
    }
}

/// ENFJ — The Protagonist. Warm amber and gold, charismatic warmth.
pub fn enfj_theme() -> Theme {
    Theme {
        bg: Color::Rgb(20, 10, 5),
        fg: Color::Rgb(245, 225, 195),
        accent: Color::Rgb(220, 140, 20),
        accent_secondary: Color::Rgb(180, 80, 40),
        header: Color::Rgb(255, 185, 45),
        muted: Color::Rgb(120, 90, 60),
        error: Color::Rgb(220, 60, 40),
        success: Color::Rgb(60, 200, 100),
        warning: Color::Rgb(240, 180, 20),
        border: Color::Rgb(95, 55, 18),
        border_active: Color::Rgb(220, 140, 20),
        thinking_dot: Color::Rgb(220, 140, 20),
        thinking_dot_bright: Color::Rgb(255, 205, 85),
    }
}

/// ENFP — The Campaigner. Bright coral and sunshine, boundless enthusiasm.
pub fn enfp_theme() -> Theme {
    Theme {
        bg: Color::Rgb(20, 10, 5),
        fg: Color::Rgb(252, 235, 205),
        accent: Color::Rgb(255, 120, 60),
        accent_secondary: Color::Rgb(255, 200, 40),
        header: Color::Rgb(255, 165, 85),
        muted: Color::Rgb(130, 100, 70),
        error: Color::Rgb(220, 50, 50),
        success: Color::Rgb(60, 210, 100),
        warning: Color::Rgb(255, 200, 40),
        border: Color::Rgb(115, 58, 18),
        border_active: Color::Rgb(255, 120, 60),
        thinking_dot: Color::Rgb(255, 120, 60),
        thinking_dot_bright: Color::Rgb(255, 205, 125),
    }
}

/// ISTJ — The Inspector. Military green and navy, steadfast duty.
pub fn istj_theme() -> Theme {
    Theme {
        bg: Color::Rgb(5, 10, 8),
        fg: Color::Rgb(200, 210, 195),
        accent: Color::Rgb(80, 140, 80),
        accent_secondary: Color::Rgb(40, 80, 120),
        header: Color::Rgb(105, 185, 105),
        muted: Color::Rgb(70, 90, 70),
        error: Color::Rgb(200, 60, 60),
        success: Color::Rgb(80, 200, 80),
        warning: Color::Rgb(200, 180, 40),
        border: Color::Rgb(28, 55, 28),
        border_active: Color::Rgb(80, 140, 80),
        thinking_dot: Color::Rgb(80, 140, 80),
        thinking_dot_bright: Color::Rgb(140, 225, 140),
    }
}

/// ISFJ — The Defender. Warm brown and parchment, nurturing earth.
pub fn isfj_theme() -> Theme {
    Theme {
        bg: Color::Rgb(18, 12, 8),
        fg: Color::Rgb(235, 215, 195),
        accent: Color::Rgb(180, 120, 60),
        accent_secondary: Color::Rgb(160, 140, 80),
        header: Color::Rgb(225, 165, 85),
        muted: Color::Rgb(105, 80, 60),
        error: Color::Rgb(210, 60, 50),
        success: Color::Rgb(80, 190, 80),
        warning: Color::Rgb(220, 175, 40),
        border: Color::Rgb(78, 52, 28),
        border_active: Color::Rgb(180, 120, 60),
        thinking_dot: Color::Rgb(180, 120, 60),
        thinking_dot_bright: Color::Rgb(242, 185, 105),
    }
}

/// ESTJ — The Executive. Bold navy and dark red, commanding authority.
pub fn estj_theme() -> Theme {
    Theme {
        bg: Color::Rgb(8, 8, 15),
        fg: Color::Rgb(220, 218, 212),
        accent: Color::Rgb(40, 80, 180),
        accent_secondary: Color::Rgb(180, 40, 40),
        header: Color::Rgb(85, 145, 225),
        muted: Color::Rgb(80, 80, 95),
        error: Color::Rgb(200, 50, 50),
        success: Color::Rgb(50, 200, 100),
        warning: Color::Rgb(210, 170, 30),
        border: Color::Rgb(28, 38, 78),
        border_active: Color::Rgb(40, 80, 180),
        thinking_dot: Color::Rgb(40, 80, 180),
        thinking_dot_bright: Color::Rgb(105, 165, 255),
    }
}

/// ESFJ — The Consul. Warm pink and friendly coral, community warmth.
pub fn esfj_theme() -> Theme {
    Theme {
        bg: Color::Rgb(20, 10, 12),
        fg: Color::Rgb(245, 225, 225),
        accent: Color::Rgb(220, 80, 120),
        accent_secondary: Color::Rgb(240, 140, 80),
        header: Color::Rgb(255, 125, 155),
        muted: Color::Rgb(120, 80, 95),
        error: Color::Rgb(220, 50, 70),
        success: Color::Rgb(60, 200, 110),
        warning: Color::Rgb(240, 175, 40),
        border: Color::Rgb(98, 38, 58),
        border_active: Color::Rgb(220, 80, 120),
        thinking_dot: Color::Rgb(220, 80, 120),
        thinking_dot_bright: Color::Rgb(255, 165, 195),
    }
}

/// ISTP — The Virtuoso. Steel grey and gunmetal, mechanical precision.
pub fn istp_theme() -> Theme {
    Theme {
        bg: Color::Rgb(8, 10, 10),
        fg: Color::Rgb(200, 208, 200),
        accent: Color::Rgb(140, 160, 160),
        accent_secondary: Color::Rgb(100, 120, 100),
        header: Color::Rgb(185, 205, 205),
        muted: Color::Rgb(80, 92, 85),
        error: Color::Rgb(200, 70, 60),
        success: Color::Rgb(80, 195, 100),
        warning: Color::Rgb(200, 185, 50),
        border: Color::Rgb(48, 58, 55),
        border_active: Color::Rgb(140, 160, 160),
        thinking_dot: Color::Rgb(140, 160, 160),
        thinking_dot_bright: Color::Rgb(212, 235, 235),
    }
}

/// ISFP — The Adventurer. Warm sand and earth tones, quiet artistry.
pub fn isfp_theme() -> Theme {
    Theme {
        bg: Color::Rgb(15, 12, 8),
        fg: Color::Rgb(228, 218, 202),
        accent: Color::Rgb(180, 150, 80),
        accent_secondary: Color::Rgb(140, 100, 80),
        header: Color::Rgb(225, 195, 105),
        muted: Color::Rgb(112, 96, 75),
        error: Color::Rgb(210, 65, 55),
        success: Color::Rgb(80, 195, 95),
        warning: Color::Rgb(215, 185, 50),
        border: Color::Rgb(78, 62, 38),
        border_active: Color::Rgb(180, 150, 80),
        thinking_dot: Color::Rgb(180, 150, 80),
        thinking_dot_bright: Color::Rgb(242, 212, 132),
    }
}

/// ESTP — The Entrepreneur. Bold red and energetic orange, live action.
pub fn estp_theme() -> Theme {
    Theme {
        bg: Color::Rgb(20, 5, 5),
        fg: Color::Rgb(242, 222, 202),
        accent: Color::Rgb(220, 50, 30),
        accent_secondary: Color::Rgb(240, 140, 0),
        header: Color::Rgb(255, 85, 55),
        muted: Color::Rgb(120, 70, 60),
        error: Color::Rgb(255, 40, 40),
        success: Color::Rgb(60, 210, 80),
        warning: Color::Rgb(240, 160, 20),
        border: Color::Rgb(98, 28, 18),
        border_active: Color::Rgb(220, 50, 30),
        thinking_dot: Color::Rgb(220, 50, 30),
        thinking_dot_bright: Color::Rgb(255, 125, 85),
    }
}

/// ESFP — The Entertainer. Bright yellow and hot pink, pure fun.
pub fn esfp_theme() -> Theme {
    Theme {
        bg: Color::Rgb(20, 10, 15),
        fg: Color::Rgb(252, 242, 212),
        accent: Color::Rgb(240, 180, 0),
        accent_secondary: Color::Rgb(220, 60, 140),
        header: Color::Rgb(255, 225, 45),
        muted: Color::Rgb(122, 102, 82),
        error: Color::Rgb(220, 50, 80),
        success: Color::Rgb(60, 210, 100),
        warning: Color::Rgb(240, 200, 20),
        border: Color::Rgb(98, 68, 18),
        border_active: Color::Rgb(240, 180, 0),
        thinking_dot: Color::Rgb(240, 180, 0),
        thinking_dot_bright: Color::Rgb(255, 242, 85),
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

/// Daltonized theme — colorblind-friendly palette.
///
/// Avoids red/green-only distinctions.  Uses blue, orange, and contrasting
/// greys so all three common dichromacy types (protanopia, deuteranopia,
/// tritanopia) can distinguish the semantic colors.
pub fn daltonized_theme() -> Theme {
    Theme {
        bg: Color::Rgb(10, 12, 18),
        fg: Color::Rgb(210, 215, 225),
        // Accent: vivid blue (safe for all types)
        accent: Color::Rgb(0, 120, 210),
        // Secondary accent: orange (distinguishable from blue)
        accent_secondary: Color::Rgb(220, 130, 0),
        header: Color::Rgb(80, 180, 255),
        muted: Color::Rgb(90, 100, 115),
        // Error: orange-red with brightness bump (not pure red)
        error: Color::Rgb(220, 100, 0),
        // Success: blue (not green — safe for deuteranopia)
        success: Color::Rgb(0, 160, 240),
        // Warning: yellow with high luminance
        warning: Color::Rgb(240, 210, 0),
        border: Color::Rgb(40, 50, 70),
        border_active: Color::Rgb(0, 120, 210),
        thinking_dot: Color::Rgb(0, 120, 210),
        thinking_dot_bright: Color::Rgb(80, 180, 255),
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
