//! Reasoning controls for OpenAI-compatible backends (#123).
//!
//! Anthropic and the Codex Responses API each have a first-class reasoning
//! knob, wired in their own adapters. OpenAI-compatible servers do not agree
//! on one: the switch is either a top-level `reasoning_effort` field or a key
//! inside `chat_template_kwargs`, which is passed verbatim into the model's
//! Jinja chat template — so the name depends on the template, not the server.
//!
//! Rather than hardcode one backend's spelling, this module is data-driven:
//! the operator declares where the level goes and how the canonical ladder
//! maps onto the tiers their model accepts.
//!
//! # Findings from a live vLLM DeepSeek-V4-Flash server
//!
//! These drove the design and are worth knowing before changing defaults:
//!
//! * **`chat_template_kwargs` is unvalidated.** An unknown key returns HTTP
//!   200 and is silently ignored — a typo produces no error and no reasoning.
//!   The top-level field, by contrast, is validated and rejects an unknown
//!   tier with a 400 that enumerates the vocabulary. Prefer
//!   [`ReasoningMode::TopLevel`] where the server supports it.
//! * **`reasoning_effort` inside `chat_template_kwargs` is inert on its own.**
//!   It only takes effect alongside `thinking: true` (or its exact alias
//!   `enable_thinking: true`). The top-level field is the opposite: it implies
//!   thinking on. The two paths are NOT interchangeable, which is why `kwargs`
//!   exists as a separate field.
//! * **Not every tier changes the prompt.** On that template only
//!   `high`/`xhigh`/`max` inject a preamble; `low` and `medium` are
//!   indistinguishable from plain thinking-on. Effort selects a preamble, not
//!   a token budget — do not present it as a length dial.
//!
//! The default is [`ReasoningMode::Off`], which sends nothing. The `local`
//! provider also serves Ollama and llama.cpp, where an unexpected top-level
//! field can be a hard 400, so reasoning is opt-in per deployment.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

/// Where the effort level is written on an OpenAI-compatible request.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReasoningMode {
    /// Send no reasoning fields at all. Preserves pre-#123 wire bytes.
    #[default]
    Off,
    /// Write the level as a top-level `reasoning_effort` sibling of `model`
    /// and `messages`. Validated server-side on vLLM.
    TopLevel,
    /// Write the level inside `chat_template_kwargs`, for templates that read
    /// it. Requires `kwargs` to carry whatever switch turns thinking on.
    ChatTemplateKwargs,
}

/// Declarative mapping from archon's effort ladder onto a backend's own
/// reasoning controls.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ReasoningConfig {
    /// Where the level is written. Defaults to [`ReasoningMode::Off`].
    pub mode: ReasoningMode,
    /// Field name that receives the mapped level. Empty means "write static
    /// `kwargs` only, no per-turn level".
    pub effort_key: String,
    /// Static keys merged into `chat_template_kwargs` on every request — e.g.
    /// `{ thinking = true }`, without which a DeepSeek template ignores the
    /// effort key entirely. Applied in both non-`Off` modes so a top-level
    /// deployment can still force a template switch.
    pub kwargs: BTreeMap<String, Value>,
    /// Canonical level (`low`/`medium`/`high`/`max`) to the backend's own
    /// vocabulary. A level absent from this map is not sent at all, which is
    /// how a backend with a shorter ladder opts out of the top rungs.
    pub effort_map: BTreeMap<String, String>,
}

impl Default for ReasoningConfig {
    fn default() -> Self {
        Self {
            mode: ReasoningMode::Off,
            effort_key: "reasoning_effort".to_string(),
            kwargs: BTreeMap::new(),
            effort_map: ["low", "medium", "high", "max"]
                .iter()
                .map(|tier| ((*tier).to_string(), (*tier).to_string()))
                .collect(),
        }
    }
}

impl ReasoningConfig {
    /// Whether this config would write anything to a request body.
    pub fn is_active(&self) -> bool {
        self.mode != ReasoningMode::Off
    }

    /// Translate a canonical effort level into the backend's vocabulary.
    /// Returns `None` when the backend has no equivalent rung.
    pub fn map_effort(&self, effort: &str) -> Option<&str> {
        self.effort_map.get(effort).map(String::as_str)
    }

    /// Write the reasoning controls for `effort` into an OpenAI-shaped body.
    ///
    /// A no-op when the mode is [`ReasoningMode::Off`], when `body` is not a
    /// JSON object, or when the level has no mapping. Existing keys are
    /// preserved: `chat_template_kwargs` supplied by another layer is merged
    /// into rather than replaced.
    pub fn apply(&self, body: &mut Value, effort: Option<&str>) {
        if !self.is_active() {
            return;
        }
        let Some(obj) = body.as_object_mut() else {
            return;
        };
        let mapped = effort
            .and_then(|level| self.map_effort(level))
            .map(|level| Value::String(level.to_string()));

        match self.mode {
            ReasoningMode::Off => unreachable!("guarded by is_active above"),
            ReasoningMode::TopLevel => {
                if let Some(level) = mapped
                    && !self.effort_key.is_empty()
                {
                    obj.insert(self.effort_key.clone(), level);
                }
                if !self.kwargs.is_empty() {
                    let target = template_kwargs_mut(obj);
                    merge_static_kwargs(target, &self.kwargs);
                }
            }
            ReasoningMode::ChatTemplateKwargs => {
                let target = template_kwargs_mut(obj);
                merge_static_kwargs(target, &self.kwargs);
                if let Some(level) = mapped
                    && !self.effort_key.is_empty()
                {
                    target.insert(self.effort_key.clone(), level);
                }
            }
        }
    }
}

/// Borrow `body["chat_template_kwargs"]` as an object, creating it if absent
/// and replacing it if some earlier layer left a non-object there.
fn template_kwargs_mut(obj: &mut Map<String, Value>) -> &mut Map<String, Value> {
    let entry = obj
        .entry("chat_template_kwargs")
        .or_insert_with(|| Value::Object(Map::new()));
    if !entry.is_object() {
        *entry = Value::Object(Map::new());
    }
    entry
        .as_object_mut()
        .expect("just ensured chat_template_kwargs is an object")
}

/// Merge static kwargs without clobbering keys already present.
fn merge_static_kwargs(target: &mut Map<String, Value>, kwargs: &BTreeMap<String, Value>) {
    for (key, value) in kwargs {
        target.insert(key.clone(), value.clone());
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn body() -> Value {
        json!({"model": "m", "messages": [], "max_tokens": 16})
    }

    fn cfg(mode: ReasoningMode) -> ReasoningConfig {
        ReasoningConfig {
            mode,
            ..ReasoningConfig::default()
        }
    }

    #[test]
    fn default_is_off_and_writes_nothing() {
        let config = ReasoningConfig::default();
        assert!(!config.is_active());
        let mut b = body();
        let before = b.clone();
        config.apply(&mut b, Some("max"));
        assert_eq!(b, before, "Off must preserve pre-#123 wire bytes exactly");
    }

    #[test]
    fn top_level_writes_validated_field() {
        let mut b = body();
        cfg(ReasoningMode::TopLevel).apply(&mut b, Some("high"));
        assert_eq!(b["reasoning_effort"], "high");
        assert!(b.get("chat_template_kwargs").is_none());
    }

    #[test]
    fn chat_template_kwargs_nests_the_level() {
        let mut b = body();
        cfg(ReasoningMode::ChatTemplateKwargs).apply(&mut b, Some("max"));
        assert_eq!(b["chat_template_kwargs"]["reasoning_effort"], "max");
    }

    /// A DeepSeek template ignores `reasoning_effort` unless thinking is
    /// switched on in the same kwargs bag — verified against a live server.
    /// The static kwargs must therefore land alongside the level.
    #[test]
    fn static_kwargs_accompany_the_level() {
        let mut config = cfg(ReasoningMode::ChatTemplateKwargs);
        config.kwargs.insert("thinking".into(), json!(true));
        let mut b = body();
        config.apply(&mut b, Some("high"));
        assert_eq!(b["chat_template_kwargs"]["thinking"], json!(true));
        assert_eq!(b["chat_template_kwargs"]["reasoning_effort"], "high");
    }

    #[test]
    fn unmapped_level_writes_nothing_but_leaves_kwargs() {
        let mut config = cfg(ReasoningMode::TopLevel);
        config.effort_map.remove("max");
        config.kwargs.insert("thinking".into(), json!(true));
        let mut b = body();
        config.apply(&mut b, Some("max"));
        assert!(
            b.get("reasoning_effort").is_none(),
            "a backend without the top rung must not receive it"
        );
        assert_eq!(b["chat_template_kwargs"]["thinking"], json!(true));
    }

    #[test]
    fn remaps_onto_a_backend_vocabulary() {
        let mut config = cfg(ReasoningMode::TopLevel);
        config.effort_map.insert("max".into(), "xhigh".into());
        let mut b = body();
        config.apply(&mut b, Some("max"));
        assert_eq!(b["reasoning_effort"], "xhigh");
    }

    #[test]
    fn empty_effort_key_writes_only_static_kwargs() {
        let mut config = cfg(ReasoningMode::ChatTemplateKwargs);
        config.effort_key = String::new();
        config.kwargs.insert("thinking".into(), json!(true));
        let mut b = body();
        config.apply(&mut b, Some("high"));
        assert_eq!(b["chat_template_kwargs"]["thinking"], json!(true));
        assert!(b["chat_template_kwargs"].get("reasoning_effort").is_none());
    }

    #[test]
    fn merges_into_existing_kwargs_rather_than_replacing() {
        let mut b = body();
        b["chat_template_kwargs"] = json!({"preserved": 1});
        let mut config = cfg(ReasoningMode::ChatTemplateKwargs);
        config.kwargs.insert("thinking".into(), json!(true));
        config.apply(&mut b, Some("low"));
        assert_eq!(b["chat_template_kwargs"]["preserved"], json!(1));
        assert_eq!(b["chat_template_kwargs"]["thinking"], json!(true));
    }

    #[test]
    fn non_object_kwargs_is_replaced_not_panicked_on() {
        let mut b = body();
        b["chat_template_kwargs"] = json!("nonsense");
        cfg(ReasoningMode::ChatTemplateKwargs).apply(&mut b, Some("low"));
        assert_eq!(b["chat_template_kwargs"]["reasoning_effort"], "low");
    }

    #[test]
    fn absent_effort_writes_no_level() {
        let mut b = body();
        cfg(ReasoningMode::TopLevel).apply(&mut b, None);
        assert!(b.get("reasoning_effort").is_none());
    }

    #[test]
    fn non_object_body_is_ignored() {
        let mut b = json!("not an object");
        cfg(ReasoningMode::TopLevel).apply(&mut b, Some("high"));
        assert_eq!(b, json!("not an object"));
    }

    #[test]
    fn round_trips_through_toml() {
        let toml_src = r#"
mode = "chat_template_kwargs"
effort_key = "reasoning_effort"
kwargs = { thinking = true }
[effort_map]
low = "low"
medium = "medium"
high = "high"
max = "xhigh"
"#;
        let config: ReasoningConfig = toml::from_str(toml_src).expect("config should parse");
        assert_eq!(config.mode, ReasoningMode::ChatTemplateKwargs);
        assert_eq!(config.map_effort("max"), Some("xhigh"));
        assert_eq!(config.kwargs.get("thinking"), Some(&json!(true)));
    }
}
