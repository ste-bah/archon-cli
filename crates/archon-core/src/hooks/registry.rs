use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::Mutex;

use serde::Deserialize;

use super::condition;
use super::executor;
use super::types::{HookError, HookEvent, HookMatcher, HookResult};

// ---------------------------------------------------------------------------
// Internal entry: one HookMatcher bound to a source (plugin name or None)
// ---------------------------------------------------------------------------

struct HookEntry {
    source: Option<String>,
    matcher: HookMatcher,
}

// ---------------------------------------------------------------------------
// HookRegistry — thread-safe registry of hook matchers indexed by event
// ---------------------------------------------------------------------------

/// Registry of hook matchers, organized by `HookEvent`.
///
/// Loaded once at startup from `.claude/settings.json` and optionally
/// extended at runtime by plugins via `register_matchers`.
pub struct HookRegistry {
    entries: HashMap<HookEvent, Vec<HookEntry>>,
    /// Tracks `once: true` hooks that have already fired (event:source:cmd).
    once_fired: Mutex<HashSet<String>>,
}

// ---------------------------------------------------------------------------
// Deserialization helper for settings.json
// ---------------------------------------------------------------------------

#[derive(Deserialize, Default)]
struct SettingsJson {
    #[serde(default)]
    hooks: HashMap<HookEvent, Vec<HookMatcher>>,
}

// ---------------------------------------------------------------------------
// Implementation
// ---------------------------------------------------------------------------

impl HookRegistry {
    /// Create an empty registry.
    pub fn new() -> Self {
        Self {
            entries: HashMap::new(),
            once_fired: Mutex::new(HashSet::new()),
        }
    }

    /// Parse a `.claude/settings.json` string and populate a registry from
    /// the top-level `"hooks"` field.
    ///
    /// Returns `Err` only on JSON parse failure — missing `"hooks"` key
    /// produces an empty registry.
    pub fn load_from_settings_json(json: &str) -> Result<Self, HookError> {
        let settings: SettingsJson = serde_json::from_str(json)
            .map_err(|e| HookError::JsonError(format!("settings.json parse error: {e}")))?;

        let mut registry = Self::new();
        for (event, matchers) in settings.hooks {
            registry.register_matchers(event, matchers, None);
        }
        Ok(registry)
    }

    /// Register a batch of `HookMatcher` entries for `event`, tagging them
    /// with an optional `source` identifier (e.g. a plugin name).
    ///
    /// Load order is execution order — no priority field.
    pub fn register_matchers(
        &mut self,
        event: HookEvent,
        matchers: Vec<HookMatcher>,
        source: Option<&str>,
    ) {
        let entries = self.entries.entry(event).or_default();
        for matcher in matchers {
            entries.push(HookEntry {
                source: source.map(str::to_owned),
                matcher,
            });
        }
    }

    /// Execute all hooks registered for `event` against `input`.
    ///
    /// Hooks run in registration order. Processing stops immediately if any
    /// hook returns exit code 2 (Block). All other non-zero exit codes are
    /// treated as non-blocking failures (logged, execution continues).
    ///
    /// Returns `HookResult::Allow` unless a hook explicitly blocks.
    pub async fn execute_hooks(
        &self,
        event: HookEvent,
        input: serde_json::Value,
        cwd: &Path,
        session_id: &str,
    ) -> HookResult {
        let entries = match self.entries.get(&event) {
            Some(e) => e,
            None => return HookResult::Allow,
        };

        let event_name = event.to_string();

        for entry in entries {
            // Apply HookMatcher.matcher filter against tool_name in input.
            if let Some(ref matcher_str) = entry.matcher.matcher {
                let tool_name = input
                    .get("tool_name")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                if !matcher_matches(matcher_str, tool_name) {
                    continue;
                }
            }

            for hook in &entry.matcher.hooks {
                // Evaluate if_condition filter.
                if let Some(ref cond) = hook.if_condition
                    && !condition::evaluate(cond, &input)
                {
                    continue;
                }

                // Check once: skip if this hook has already fired.
                let once_key = make_once_key(&event_name, &entry.source, &hook.command);
                if hook.once == Some(true) {
                    let already_fired = self
                        .once_fired
                        .lock()
                        .unwrap_or_else(|p| p.into_inner())
                        .contains(&once_key);
                    if already_fired {
                        continue;
                    }
                }

                // Execute the hook command.
                let result =
                    executor::execute_hook(hook, &input, cwd, session_id, &event_name).await;

                // Mark once-hooks as fired after execution.
                if hook.once == Some(true)
                    && let Ok(mut fired) = self.once_fired.lock()
                {
                    fired.insert(once_key);
                }

                match result {
                    HookResult::Block { reason } => {
                        return HookResult::Block { reason };
                    }
                    HookResult::Allow => {} // continue to next hook
                }
            }
        }

        HookResult::Allow
    }
}

impl Default for HookRegistry {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Check if a HookMatcher.matcher string matches a tool name.
/// Simple equality check; "*" matches everything.
fn matcher_matches(matcher: &str, tool_name: &str) -> bool {
    matcher == "*" || matcher == tool_name
}

fn make_once_key(event_name: &str, source: &Option<String>, command: &str) -> String {
    format!("{event_name}:{}:{command}", source.as_deref().unwrap_or(""))
}
