//! Output styles — behavioral presets injected into the system prompt.
//!
//! Output styles change *how the model responds* (verbosity, tone, explanation
//! depth) by appending a style-specific instruction string to the system prompt
//! before each API call.  They are NOT TUI rendering themes; no color, bold, or
//! italic fields exist here.

use std::collections::HashMap;

// ---------------------------------------------------------------------------
// OutputStyleSource
// ---------------------------------------------------------------------------

/// Where a style was loaded from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OutputStyleSource {
    /// Shipped with Archon (always available).
    BuiltIn,
    /// Loaded from `~/.claude/output-styles/` at startup.
    Config,
    /// Provided by a plugin via its `output-styles/` directory.
    Plugin,
}

// ---------------------------------------------------------------------------
// OutputStyleConfig
// ---------------------------------------------------------------------------

/// Configuration for one output style.
///
/// The `prompt` field, when `Some`, is appended to the base system prompt
/// before each API call.  When `None` (as with the built-in `"default"`
/// style), no injection occurs.
#[derive(Debug, Clone)]
pub struct OutputStyleConfig {
    /// Unique style identifier used in config and CLI flag.
    pub name: String,
    /// Human-readable description shown in help / listings.
    pub description: String,
    /// Text appended to the system prompt.  `None` → no injection.
    pub prompt: Option<String>,
    /// Where this style originated.
    pub source: OutputStyleSource,
    /// If `true`, the style's prompt is appended alongside the default coding
    /// instructions rather than replacing them.
    pub keep_coding_instructions: Option<bool>,
    /// If `true`, this style is automatically applied when its owning plugin
    /// is enabled.  Only meaningful for `Plugin`-source styles.
    pub force_for_plugin: Option<bool>,
}

impl OutputStyleConfig {
    /// Return a copy of `base_prompt` with this style's prompt appended.
    ///
    /// If `self.prompt` is `None`, returns `base_prompt` unchanged.
    pub fn inject_into(&self, base_prompt: &str) -> String {
        match &self.prompt {
            None => base_prompt.to_owned(),
            Some(injection) => format!("{base_prompt}\n\n{injection}"),
        }
    }
}

// ---------------------------------------------------------------------------
// OutputStyleRegistry
// ---------------------------------------------------------------------------

/// Registry of named output styles.
///
/// Pre-populated with five built-in styles at construction.  Additional styles
/// can be registered via `register()` (from config files or plugins).
pub struct OutputStyleRegistry {
    styles: HashMap<String, OutputStyleConfig>,
}

impl OutputStyleRegistry {
    /// Create a new registry pre-loaded with built-in styles.
    pub fn new() -> Self {
        let mut reg = Self {
            styles: HashMap::new(),
        };
        for style in builtin_styles() {
            reg.styles.insert(style.name.clone(), style);
        }
        reg
    }

    /// Look up a style by name.  Returns `None` if not registered.
    pub fn get(&self, name: &str) -> Option<&OutputStyleConfig> {
        self.styles.get(name)
    }

    /// Look up a style by name, falling back to `"default"` if not found.
    ///
    /// Logs a warning when the fallback is used.
    pub fn get_or_default(&self, name: &str) -> &OutputStyleConfig {
        if let Some(style) = self.styles.get(name) {
            return style;
        }
        tracing::warn!(
            output_style = name,
            "unknown output style, falling back to default"
        );
        self.styles
            .get("default")
            .expect("built-in 'default' style must always be present")
    }

    /// Register (or overwrite) a style.
    pub fn register(&mut self, style: OutputStyleConfig) {
        self.styles.insert(style.name.clone(), style);
    }

    /// Remove all styles registered with the given source, typically used
    /// when a plugin is unloaded.
    pub fn clear_by_source(&mut self, source: &OutputStyleSource) {
        self.styles.retain(|_, v| &v.source != source);
    }

    /// Return a sorted list of all registered style names.
    pub fn list(&self) -> Vec<String> {
        let mut names: Vec<String> = self.styles.keys().cloned().collect();
        names.sort();
        names
    }

    /// Return the first plugin-provided style with `force_for_plugin: true`,
    /// if any.  When multiple plugins force styles, the first one encountered
    /// in hash order wins (and a warning is logged for subsequent ones).
    pub fn forced_plugin_style(&self) -> Option<&OutputStyleConfig> {
        let mut found: Option<&OutputStyleConfig> = None;
        for style in self.styles.values() {
            if style.source == OutputStyleSource::Plugin
                && style.force_for_plugin == Some(true)
            {
                if found.is_some() {
                    tracing::warn!(
                        style = style.name.as_str(),
                        "multiple plugins force an output style; using the first one loaded"
                    );
                } else {
                    found = Some(style);
                }
            }
        }
        found
    }
}

impl Default for OutputStyleRegistry {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Built-in style definitions
// ---------------------------------------------------------------------------

fn builtin_styles() -> Vec<OutputStyleConfig> {
    vec![
        OutputStyleConfig {
            name: "default".into(),
            description: "Claude's default behavior — no system prompt injection.".into(),
            prompt: None,
            source: OutputStyleSource::BuiltIn,
            keep_coding_instructions: None,
            force_for_plugin: None,
        },
        OutputStyleConfig {
            name: "Explanatory".into(),
            description: "Adds educational insights before and after code blocks.".into(),
            prompt: Some(
                "Before presenting any code, briefly explain the key concept or pattern being \
                 used. After the code, add a short \"How it works\" section highlighting the \
                 most important lines. Prioritise clarity for learners over brevity."
                    .into(),
            ),
            source: OutputStyleSource::BuiltIn,
            keep_coding_instructions: Some(true),
            force_for_plugin: None,
        },
        OutputStyleConfig {
            name: "Learning".into(),
            description: "Pauses for hands-on practice exercises after explanations.".into(),
            prompt: Some(
                "After explaining each concept or completing each task, suggest a short \
                 hands-on exercise the user can try themselves. Keep exercises focused and \
                 achievable in under five minutes. Ask if the user would like to try before \
                 continuing."
                    .into(),
            ),
            source: OutputStyleSource::BuiltIn,
            keep_coding_instructions: Some(true),
            force_for_plugin: None,
        },
        OutputStyleConfig {
            name: "Formal".into(),
            description: "Structured, professional tone suited for technical documentation.".into(),
            prompt: Some(
                "Respond in a formal, professional register. Use precise technical \
                 terminology, avoid colloquialisms, and structure responses with clear \
                 headings and numbered lists where appropriate. Cite relevant standards \
                 or best practices when applicable."
                    .into(),
            ),
            source: OutputStyleSource::BuiltIn,
            keep_coding_instructions: Some(true),
            force_for_plugin: None,
        },
        OutputStyleConfig {
            name: "Concise".into(),
            description: "Brief, minimal explanations — code-first, prose-minimal.".into(),
            prompt: Some(
                "Be extremely concise. Lead with code or the direct answer. Omit \
                 preambles, summaries, and filler phrases. If an explanation is necessary, \
                 keep it to one sentence. Prefer terse responses over thorough ones."
                    .into(),
            ),
            source: OutputStyleSource::BuiltIn,
            keep_coding_instructions: Some(false),
            force_for_plugin: None,
        },
    ]
}
