//! Theme registry for TASK-CLI-315.
//!
//! Holds all named themes (built-in + custom) and resolves `"auto"` to the
//! appropriate dark/light theme based on system detection.

use std::collections::BTreeMap;

use crate::theme::{available_themes, daltonized_theme, dark_theme, light_theme, theme_by_name, Theme};

// ---------------------------------------------------------------------------
// ThemeRegistry
// ---------------------------------------------------------------------------

/// Registry of all available themes, keyed by lowercase name.
pub struct ThemeRegistry {
    themes: BTreeMap<String, Theme>,
}

impl ThemeRegistry {
    /// Create a new registry pre-loaded with all 22 built-in themes plus
    /// `"daltonized"` and `"auto"` (a snapshot of the detected system theme).
    pub fn new() -> Self {
        let mut themes = BTreeMap::new();

        // 22 built-in named themes
        for name in available_themes() {
            if let Some(theme) = theme_by_name(name) {
                themes.insert(name.to_string(), theme);
            }
        }

        // daltonized is listed separately from the 22 built-ins
        themes.insert("daltonized".to_string(), daltonized_theme());

        // "auto" stores the currently detected system theme so that names()
        // includes it and it is hot-swappable like any other named theme.
        themes.insert("auto".to_string(), detect_system_theme());

        Self { themes }
    }

    /// Look up a theme by exact name.  Returns `Some` for all registered names
    /// including `"auto"` and `"daltonized"`.  Returns `None` for unknown names.
    pub fn get(&self, name: &str) -> Option<&Theme> {
        self.themes.get(&name.to_lowercase())
    }

    /// Resolve a theme name, handling `"auto"` and unknown names.
    ///
    /// - `"auto"` → detected dark/light theme
    /// - known name → that theme
    /// - unknown name → `dark` fallback (with warning log)
    pub fn resolve(&self, name: &str) -> Theme {
        let lower = name.to_lowercase();
        if let Some(theme) = self.themes.get(&lower) {
            return theme.clone();
        }
        tracing::warn!(theme = name, "unknown theme, falling back to 'dark'");
        dark_theme()
    }

    /// Register (or overwrite) a named theme.  The name is lowercased.
    pub fn register(&mut self, name: &str, theme: Theme) {
        self.themes.insert(name.to_lowercase(), theme);
    }

    /// Return a sorted list of all registered theme names (includes `"auto"` and `"daltonized"`).
    pub fn names(&self) -> Vec<String> {
        self.themes.keys().cloned().collect()
    }
}

impl Default for ThemeRegistry {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// System theme detection
// ---------------------------------------------------------------------------

/// Detect whether the system is using a dark or light terminal theme.
///
/// Detection order:
/// 1. `ARCHON_THEME_PREFER` env var (`"light"` → light, anything else → dark)
/// 2. `COLORFGBG` env var — `"15;0"` = dark bg → dark; `"0;15"` = light bg → light
/// 3. `TERM_PROGRAM` — `"iTerm.app"` respects `ITERM_PROFILE` when set
/// 4. Default: `dark`
pub fn detect_system_theme() -> Theme {
    // Allow explicit override for testing
    if let Ok(val) = std::env::var("ARCHON_THEME_PREFER") {
        if val.to_lowercase() == "light" {
            return light_theme();
        }
        return dark_theme();
    }

    // COLORFGBG: foreground;background (legacy xterm convention)
    // "15;0" → light fg, dark bg → dark theme
    // "0;15" → dark fg, light bg → light theme
    if let Ok(val) = std::env::var("COLORFGBG") {
        let parts: Vec<&str> = val.split(';').collect();
        if let Some(bg) = parts.last() {
            match *bg {
                "15" | "7" => return light_theme(), // light background
                _ => return dark_theme(),
            }
        }
    }

    // Default to dark — the most common terminal background
    dark_theme()
}
