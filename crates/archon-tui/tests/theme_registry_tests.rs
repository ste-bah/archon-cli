//! Tests for TASK-CLI-315: ThemeRegistry, auto/daltonized themes, hex parsing, fallback.

use archon_tui::theme::{Theme, available_themes, daltonized_theme, theme_by_name};
use archon_tui::theme_registry::ThemeRegistry;
use ratatui::style::Color;

// ---------------------------------------------------------------------------
// existing 22 themes still load
// ---------------------------------------------------------------------------

#[test]
fn all_22_builtin_themes_load() {
    let names = available_themes();
    assert_eq!(
        names.len(),
        22,
        "should have exactly 22 built-in theme names"
    );
    for name in names {
        assert!(
            theme_by_name(name).is_some(),
            "theme_by_name({name:?}) must return Some"
        );
    }
}

#[test]
fn all_mbti_themes_present() {
    let mbti = [
        "intj", "intp", "entj", "entp", "infj", "infp", "enfj", "enfp", "istj", "istp", "estj",
        "estp", "isfj", "isfp", "esfj", "esfp",
    ];
    for name in mbti {
        assert!(
            theme_by_name(name).is_some(),
            "MBTI theme {name} must exist"
        );
    }
}

#[test]
fn utility_themes_present() {
    for name in &["dark", "light", "ocean", "fire", "forest", "mono"] {
        assert!(
            theme_by_name(name).is_some(),
            "utility theme {name} must exist"
        );
    }
}

// ---------------------------------------------------------------------------
// auto theme
// ---------------------------------------------------------------------------

#[test]
fn auto_theme_resolves_without_panic() {
    let reg = ThemeRegistry::new();
    // resolve("auto") must return either dark or light — no panic, no None
    let theme = reg.resolve("auto");
    // dark theme bg is black; light theme bg is white — both are valid ratatui Colors
    let _ = theme.bg;
    let _ = theme.fg;
}

#[test]
fn auto_theme_is_dark_or_light() {
    let reg = ThemeRegistry::new();
    let auto = reg.resolve("auto");
    let dark = theme_by_name("dark").unwrap();
    let light = theme_by_name("light").unwrap();
    // auto must equal one of them
    assert!(
        auto == dark || auto == light,
        "auto must resolve to dark or light theme"
    );
}

// ---------------------------------------------------------------------------
// daltonized theme
// ---------------------------------------------------------------------------

#[test]
fn daltonized_theme_loads() {
    let theme = daltonized_theme();
    // Must have distinct bg/fg
    assert_ne!(theme.bg, theme.fg);
}

#[test]
fn daltonized_theme_via_theme_by_name() {
    assert!(
        theme_by_name("daltonized").is_some(),
        "theme_by_name('daltonized') must work"
    );
}

#[test]
fn daltonized_no_pure_red_success() {
    // In a colorblind-friendly theme, success should not be pure red
    let theme = daltonized_theme();
    assert_ne!(
        theme.success,
        Color::Red,
        "daltonized success must not be pure Red"
    );
    assert_ne!(
        theme.success,
        Color::Rgb(255, 0, 0),
        "daltonized success must not be pure RGB red"
    );
}

#[test]
fn daltonized_no_pure_red_error_vs_success_clash() {
    // error and success must be visually distinct in daltonized mode
    let theme = daltonized_theme();
    assert_ne!(
        theme.error, theme.success,
        "error and success must be different colors"
    );
}

#[test]
fn registry_contains_daltonized() {
    let reg = ThemeRegistry::new();
    let t = reg.get("daltonized");
    assert!(t.is_some(), "registry must contain 'daltonized'");
}

// ---------------------------------------------------------------------------
// Theme::from_hex
// ---------------------------------------------------------------------------

#[test]
fn from_hex_parses_rgb() {
    let color = Theme::from_hex("#1E1E2E").expect("valid hex");
    assert_eq!(color, Color::Rgb(0x1E, 0x1E, 0x2E));
}

#[test]
fn from_hex_parses_without_hash() {
    let color = Theme::from_hex("CBA6F7").expect("hex without #");
    assert_eq!(color, Color::Rgb(0xCB, 0xA6, 0xF7));
}

#[test]
fn from_hex_lowercase() {
    let color = Theme::from_hex("#ff8800").expect("lowercase hex");
    assert_eq!(color, Color::Rgb(0xFF, 0x88, 0x00));
}

#[test]
fn from_hex_invalid_returns_none() {
    assert!(Theme::from_hex("#ZZZZZZ").is_none());
    assert!(Theme::from_hex("not-a-hex").is_none());
    assert!(Theme::from_hex("").is_none());
    assert!(Theme::from_hex("#FFF").is_none()); // too short
}

// ---------------------------------------------------------------------------
// ThemeRegistry
// ---------------------------------------------------------------------------

#[test]
fn registry_new_has_24_themes() {
    let reg = ThemeRegistry::new();
    let names = reg.names();
    // 22 built-in + auto + daltonized = 24
    assert!(
        names.len() >= 24,
        "registry must have at least 24 themes, got {}",
        names.len()
    );
}

#[test]
fn registry_get_known_theme() {
    let reg = ThemeRegistry::new();
    assert!(reg.get("ocean").is_some());
    assert!(reg.get("intj").is_some());
}

#[test]
fn registry_get_unknown_returns_none() {
    let reg = ThemeRegistry::new();
    assert!(reg.get("nonexistent_theme_xyz").is_none());
}

#[test]
fn registry_resolve_unknown_falls_back_to_dark() {
    let reg = ThemeRegistry::new();
    let fallback = reg.resolve("nonexistent_theme_xyz");
    let dark = theme_by_name("dark").unwrap();
    assert_eq!(fallback, dark, "unknown theme must fall back to dark");
}

#[test]
fn registry_resolve_known_theme() {
    let reg = ThemeRegistry::new();
    let ocean = reg.resolve("ocean");
    let expected = theme_by_name("ocean").unwrap();
    assert_eq!(ocean, expected);
}

#[test]
fn registry_register_custom_theme() {
    let mut reg = ThemeRegistry::new();
    let custom = theme_by_name("mono").unwrap(); // reuse mono as "custom" for test
    reg.register("my_custom", custom.clone());
    let got = reg
        .get("my_custom")
        .expect("custom theme must be retrievable");
    assert_eq!(*got, custom);
}

#[test]
fn registry_register_from_hex_config() {
    let mut reg = ThemeRegistry::new();

    // Simulate loading a custom theme from hex config
    let bg = Theme::from_hex("#1E1E2E").unwrap();
    let fg = Theme::from_hex("#CDD6F4").unwrap();
    let accent = Theme::from_hex("#CBA6F7").unwrap();

    let custom = Theme {
        bg,
        fg,
        accent,
        accent_secondary: Theme::from_hex("#89DCEB").unwrap(),
        header: Theme::from_hex("#F5C2E7").unwrap(),
        muted: Theme::from_hex("#585B70").unwrap(),
        error: Theme::from_hex("#F38BA8").unwrap(),
        success: Theme::from_hex("#A6E3A1").unwrap(),
        warning: Theme::from_hex("#F9E2AF").unwrap(),
        border: Theme::from_hex("#313244").unwrap(),
        border_active: Theme::from_hex("#CBA6F7").unwrap(),
        thinking_dot: Theme::from_hex("#89B4FA").unwrap(),
        thinking_dot_bright: Theme::from_hex("#B4BEFE").unwrap(),
    };

    reg.register("catppuccin", custom.clone());
    let got = reg.get("catppuccin").unwrap();
    assert_eq!(got.bg, Color::Rgb(0x1E, 0x1E, 0x2E));
}

// ---------------------------------------------------------------------------
// Fallback and hot-swap
// ---------------------------------------------------------------------------

#[test]
fn resolve_returns_valid_theme_for_all_builtins() {
    let reg = ThemeRegistry::new();
    for name in available_themes() {
        let theme = reg.resolve(name);
        // All fields must be populated (not Color::Reset)
        assert_ne!(theme.bg, Color::Reset, "{name}.bg must not be Reset");
        assert_ne!(theme.fg, Color::Reset, "{name}.fg must not be Reset");
    }
}

#[test]
fn names_list_is_sorted() {
    let reg = ThemeRegistry::new();
    let names = reg.names();
    let mut sorted = names.clone();
    sorted.sort();
    assert_eq!(names, sorted, "names() should return sorted list");
}
