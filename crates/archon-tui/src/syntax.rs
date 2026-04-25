//! Syntax highlighting for code blocks using tree-sitter.
//!
//! Provides [`highlight_code`] which attempts to load a tree-sitter grammar
//! dynamically from `~/.local/share/archon/grammars/` and highlight the code.
//! When no grammar is available, returns `None` so the caller can fall back.
//!
//! [`render_plain_code`] renders code as monospace text with an optional
//! dim language label — used when no grammar is found.

use std::path::PathBuf;
use std::sync::Mutex;

use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};

/// Capture name → ratatui color mapping for tree-sitter highlights.
///
/// Exposed publicly so tests can verify the mapping exists.
pub static CAPTURE_COLORS: &[(&str, Color)] = &[
    ("keyword", Color::Magenta),
    ("string", Color::Green),
    ("comment", Color::DarkGray),
    ("function", Color::Blue),
    ("function.builtin", Color::Blue),
    ("type", Color::Yellow),
    ("type.builtin", Color::Yellow),
    ("variable", Color::White),
    ("variable.builtin", Color::White),
    ("number", Color::Cyan),
    ("operator", Color::Red),
    ("constant", Color::Cyan),
    ("constant.builtin", Color::Cyan),
    ("property", Color::White),
    ("punctuation", Color::White),
    ("punctuation.bracket", Color::White),
    ("punctuation.delimiter", Color::White),
];

/// The capture names used when configuring `HighlightConfiguration`.
///
/// Order matters: the index in this slice corresponds to the `Highlight` index
/// returned by `tree-sitter-highlight`.
const CAPTURE_NAMES: &[&str] = &[
    "keyword",
    "string",
    "comment",
    "function",
    "function.builtin",
    "type",
    "type.builtin",
    "variable",
    "variable.builtin",
    "number",
    "operator",
    "constant",
    "constant.builtin",
    "property",
    "punctuation",
    "punctuation.bracket",
    "punctuation.delimiter",
];

/// Grammar storage directory: `~/.local/share/archon/grammars/`.
pub fn grammar_dir() -> PathBuf {
    dirs::data_dir()
        .unwrap_or_else(|| PathBuf::from(".local/share"))
        .join("archon")
        .join("grammars")
}

/// Resolve the tree-sitter entry-point symbol name for a language.
///
/// Convention: the shared library exports `tree_sitter_{lang}`.
/// Some languages need a mapping (e.g. "typescript" → "tree_sitter_typescript").
fn symbol_name(lang: &str) -> String {
    let normalized = lang.replace('-', "_");
    format!("tree_sitter_{normalized}")
}

/// Attempt to load a tree-sitter `Language` from a dynamic `.so` file.
///
/// Looks for `{grammar_dir}/{lang}.so` and calls the exported
/// `tree_sitter_{lang}` function to obtain the `Language`.
///
/// # Safety
///
/// Loading arbitrary shared libraries is inherently unsafe. We trust that
/// grammar `.so` files placed in the grammar directory are valid tree-sitter
/// grammars compiled for the host platform.
fn load_language(lang: &str) -> Option<tree_sitter::Language> {
    let dir = grammar_dir();
    let lib_path = dir.join(format!("{lang}.so"));
    if !lib_path.exists() {
        tracing::debug!("grammar not found: {}", lib_path.display());
        return None;
    }

    let sym = symbol_name(lang);

    // SAFETY: We trust that the .so file in the grammar directory is a valid
    // tree-sitter grammar compiled for the current platform.
    let lib = match unsafe { libloading::Library::new(&lib_path) } {
        Ok(lib) => lib,
        Err(e) => {
            tracing::warn!("failed to load grammar library {}: {e}", lib_path.display());
            return None;
        }
    };

    // The tree-sitter convention: the library exports a function that returns
    // a *const TSLanguage (which tree_sitter::Language wraps).
    let func: libloading::Symbol<unsafe extern "C" fn() -> tree_sitter::Language> =
        match unsafe { lib.get(sym.as_bytes()) } {
            Ok(f) => f,
            Err(e) => {
                tracing::warn!("symbol '{sym}' not found in {}: {e}", lib_path.display());
                return None;
            }
        };

    let language = unsafe { func() };

    // Deliberately leak the library so the language pointer remains valid
    // for the lifetime of the process. Grammar libraries are small and we
    // only load each language once.
    std::mem::forget(lib);

    Some(language)
}

/// A lazily-initialized per-language highlighter.
///
/// We keep a `Mutex<Vec<…>>` of loaded configurations rather than a `HashMap`
/// to keep things simple and lock-free on the read path for the common case
/// where the language was already tried and is `None`.
static CONFIGS: Mutex<Vec<(String, Option<CachedConfig>)>> = Mutex::new(Vec::new());

struct CachedConfig {
    config: tree_sitter_highlight::HighlightConfiguration,
}

// SAFETY: HighlightConfiguration is Send but not marked as such upstream.
// It holds no thread-local state.
unsafe impl Send for CachedConfig {}

/// Look up or lazily create a `HighlightConfiguration` for `lang`.
///
/// Returns `None` when no grammar `.so` is available.
fn get_or_load_config(lang: &str) -> Option<()> {
    // We only use this to check whether we *can* highlight; actual
    // highlighting re-loads because HighlightConfiguration is not Sync and
    // cannot be shared across threads behind a simple Mutex without blocking.
    // For a TUI that renders on a single thread this is fine.
    let mut configs = CONFIGS.lock().ok()?;
    if let Some(entry) = configs.iter().find(|(k, _)| k == lang) {
        return entry.1.as_ref().map(|_| ());
    }
    // Try to load
    let language = load_language(lang);
    if let Some(language) = language {
        // We need highlight queries for this language. They should be in
        // {grammar_dir}/{lang}/highlights.scm
        let query_path = grammar_dir().join(lang).join("highlights.scm");
        let query = std::fs::read_to_string(&query_path).ok();
        if let Some(query) = query {
            let mut config = match tree_sitter_highlight::HighlightConfiguration::new(
                language, lang, &query, "", "",
            ) {
                Ok(c) => c,
                Err(e) => {
                    tracing::warn!("failed to create highlight config for {lang}: {e}");
                    configs.push((lang.to_string(), None));
                    return None;
                }
            };
            config.configure(CAPTURE_NAMES);
            configs.push((lang.to_string(), Some(CachedConfig { config })));
            Some(())
        } else {
            tracing::debug!(
                "highlights.scm not found for {lang} at {}",
                query_path.display()
            );
            configs.push((lang.to_string(), None));
            None
        }
    } else {
        configs.push((lang.to_string(), None));
        None
    }
}

/// Map a capture index to a ratatui `Color`.
fn color_for_capture(index: usize) -> Color {
    if index < CAPTURE_NAMES.len() {
        let name = CAPTURE_NAMES[index];
        CAPTURE_COLORS
            .iter()
            .find(|(n, _)| *n == name)
            .map(|(_, c)| *c)
            .unwrap_or(Color::White)
    } else {
        Color::White
    }
}

/// Highlight a code block using tree-sitter.
///
/// Returns `Some(Vec<Line>)` with syntax-highlighted lines if a grammar and
/// highlight queries are available for `language`. Returns `None` otherwise,
/// allowing the caller to fall back to [`render_plain_code`].
pub fn highlight_code(code: &str, language: &str) -> Option<Vec<Line<'static>>> {
    // Check if we can load a config for this language.
    get_or_load_config(language)?;

    // Re-acquire the config and run the highlighter.
    let configs = CONFIGS.lock().ok()?;
    let cached = configs.iter().find(|(k, _)| k == language)?.1.as_ref()?;

    let mut highlighter = tree_sitter_highlight::Highlighter::new();
    let events = highlighter
        .highlight(&cached.config, code.as_bytes(), None, |_| None)
        .ok()?;

    let mut lines: Vec<Line<'static>> = Vec::new();
    let mut current_spans: Vec<Span<'static>> = Vec::new();
    let mut style_stack: Vec<Style> = vec![Style::default().fg(Color::White)];

    // yield_now not needed: sync loop, not cancellable regardless.
    for event in events {
        let event = match event {
            Ok(e) => e,
            Err(_) => break,
        };
        match event {
            tree_sitter_highlight::HighlightEvent::Source { start, end } => {
                let text = &code[start..end];
                let style = style_stack.last().copied().unwrap_or_default();
                // Split on newlines to create separate Line objects
                let mut parts = text.split('\n');
                if let Some(first) = parts.next()
                    && !first.is_empty()
                {
                    current_spans.push(Span::styled(first.to_string(), style));
                }
                for part in parts {
                    lines.push(Line::from(std::mem::take(&mut current_spans)));
                    if !part.is_empty() {
                        current_spans.push(Span::styled(part.to_string(), style));
                    }
                }
            }
            tree_sitter_highlight::HighlightEvent::HighlightStart(highlight) => {
                let color = color_for_capture(highlight.0);
                style_stack.push(Style::default().fg(color));
            }
            tree_sitter_highlight::HighlightEvent::HighlightEnd => {
                if style_stack.len() > 1 {
                    style_stack.pop();
                }
            }
        }
    }

    // Flush remaining spans
    if !current_spans.is_empty() {
        lines.push(Line::from(current_spans));
    }

    Some(lines)
}

/// Render code as plain monospace text with an optional dim language label.
///
/// Used as a fallback when no tree-sitter grammar is available.
pub fn render_plain_code(code: &str, language: Option<&str>) -> Vec<Line<'static>> {
    let code_style = Style::default().fg(Color::White).bg(Color::Rgb(30, 30, 40));

    let mut lines = Vec::new();

    // Language label on the first line (dim)
    if let Some(lang) = language {
        lines.push(Line::from(Span::styled(
            format!("[{lang}]"),
            Style::default()
                .fg(Color::DarkGray)
                .add_modifier(Modifier::DIM),
        )));
    }

    // Code lines
    // yield_now not needed: sync loop, not cancellable regardless.
    for line in code.lines() {
        lines.push(Line::from(Span::styled(line.to_string(), code_style)));
    }

    // If code was empty, still add an empty code line so output is non-empty
    if code.is_empty() {
        lines.push(Line::from(Span::styled(String::new(), code_style)));
    }

    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grammar_dir_path_correct() {
        let dir = grammar_dir();
        let s = dir.to_string_lossy();
        assert!(s.contains("archon"));
        assert!(s.contains("grammars"));
    }

    #[test]
    fn symbol_name_basic() {
        assert_eq!(symbol_name("rust"), "tree_sitter_rust");
        assert_eq!(symbol_name("python"), "tree_sitter_python");
    }

    #[test]
    fn symbol_name_with_hyphens() {
        assert_eq!(symbol_name("tree-sitter"), "tree_sitter_tree_sitter");
    }

    #[test]
    fn color_for_capture_keyword() {
        // "keyword" is index 0 in CAPTURE_NAMES
        assert_eq!(color_for_capture(0), Color::Magenta);
    }

    #[test]
    fn color_for_capture_string() {
        // "string" is index 1
        assert_eq!(color_for_capture(1), Color::Green);
    }

    #[test]
    fn color_for_capture_out_of_range() {
        assert_eq!(color_for_capture(999), Color::White);
    }

    #[test]
    fn highlight_code_returns_none_for_missing_grammar() {
        let result = highlight_code("x = 1", "nonexistent_lang_xyz");
        assert!(result.is_none());
    }

    #[test]
    fn render_plain_code_with_lang() {
        let lines = render_plain_code("hello", Some("test"));
        assert!(lines.len() >= 2); // label + code
        let first_text = lines[0]
            .spans
            .iter()
            .map(|s| s.content.to_string())
            .collect::<String>();
        assert!(first_text.contains("test"));
    }

    #[test]
    fn render_plain_code_without_lang() {
        let lines = render_plain_code("hello", None);
        assert!(!lines.is_empty());
    }

    #[test]
    fn render_plain_code_multiline() {
        let lines = render_plain_code("a\nb\nc", Some("txt"));
        // 1 label + 3 code lines
        assert_eq!(lines.len(), 4);
    }

    #[test]
    fn capture_colors_has_expected_entries() {
        let expected = ["keyword", "string", "comment", "function", "type", "number"];
        for name in &expected {
            assert!(
                CAPTURE_COLORS.iter().any(|(k, _)| k == name),
                "missing capture color for {name}"
            );
        }
    }
}
