//! 7 utility (non-MBTI) theme constructors.
//!
//! Relocated from `src/theme.rs` (lines 434-574) per REM-2i. Data-only module.

use ratatui::style::Color;

use super::Theme;

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
