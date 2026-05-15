use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use serde::Deserialize;
use toml::Value;

const BUNDLED_CONTEXT_CATALOG: &str = include_str!("../resources/context.toml");

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContextWindowEntry {
    pub context_window: u64,
    pub runtime_context_budget: Option<u64>,
    pub max_output_tokens: Option<u64>,
    pub source: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct ContextCatalog {
    raw: RawCatalog,
}

impl ContextCatalog {
    pub fn bundled() -> Self {
        Self::from_toml_str(BUNDLED_CONTEXT_CATALOG).unwrap_or_default()
    }

    pub fn user_overrides(work_dir: Option<&Path>) -> Self {
        let mut merged = empty_table();
        for path in discover_context_catalog_paths(work_dir) {
            let Ok(content) = fs::read_to_string(&path) else {
                continue;
            };
            match parse_toml_value(&content) {
                Ok(value) => merged = deep_merge_toml(merged, value),
                Err(err) => tracing::warn!(
                    path = %path.display(),
                    "skipping context catalog due to parse error: {err}"
                ),
            }
        }
        Self::from_value(merged).unwrap_or_default()
    }

    pub fn load(work_dir: Option<&Path>) -> Self {
        let mut merged =
            parse_toml_value(BUNDLED_CONTEXT_CATALOG).unwrap_or_else(|_| empty_table());
        for path in discover_context_catalog_paths(work_dir) {
            let Ok(content) = fs::read_to_string(&path) else {
                continue;
            };
            match parse_toml_value(&content) {
                Ok(value) => merged = deep_merge_toml(merged, value),
                Err(err) => tracing::warn!(
                    path = %path.display(),
                    "skipping context catalog due to parse error: {err}"
                ),
            }
        }
        Self::from_value(merged).unwrap_or_default()
    }

    pub fn from_toml_str(content: &str) -> Result<Self, toml::de::Error> {
        Self::from_value(parse_toml_value(content)?)
    }

    pub fn lookup(
        &self,
        provider: &str,
        model: &str,
        active_betas: &[String],
        active_identity: Option<&str>,
    ) -> Option<ContextWindowEntry> {
        let provider = get_case_insensitive(&self.raw.providers, provider)?;
        lookup_model(provider, model, active_betas, active_identity)
    }

    pub fn lookup_any(
        &self,
        model: &str,
        active_betas: &[String],
        active_identity: Option<&str>,
    ) -> Option<ContextWindowEntry> {
        self.raw
            .providers
            .values()
            .find_map(|provider| lookup_model(provider, model, active_betas, active_identity))
    }
}

pub fn discover_context_catalog_paths(work_dir: Option<&Path>) -> Vec<PathBuf> {
    let mut paths = Vec::new();
    if let Some(config_dir) = dirs::config_dir() {
        paths.push(config_dir.join("archon").join("context.toml"));
    }
    if let Some(home) = dirs::home_dir() {
        paths.push(home.join(".archon").join("context.toml"));
    }
    if let Some(work_dir) = work_dir {
        paths.push(work_dir.join(".archon").join("context.toml"));
        paths.push(work_dir.join(".archon").join("context.local.toml"));
    }
    paths
}

fn lookup_model(
    provider: &RawProvider,
    model: &str,
    active_betas: &[String],
    active_identity: Option<&str>,
) -> Option<ContextWindowEntry> {
    let raw = get_case_insensitive(&provider.models, model)?;
    let mut best = raw
        .context_window
        .filter(|window| *window > 0)
        .map(|window| ContextWindowEntry {
            context_window: window,
            runtime_context_budget: raw.runtime_context_budget,
            max_output_tokens: raw.max_output_tokens,
            source: raw.source.clone(),
        });

    for variant in raw.variants.values() {
        if !variant_is_active(variant, active_betas, active_identity) {
            continue;
        }
        let Some(window) = variant.context_window.filter(|window| *window > 0) else {
            continue;
        };
        if best
            .as_ref()
            .is_none_or(|entry| window > entry.context_window)
        {
            best = Some(ContextWindowEntry {
                context_window: window,
                runtime_context_budget: variant.runtime_context_budget.or(raw.runtime_context_budget),
                max_output_tokens: variant.max_output_tokens.or(raw.max_output_tokens),
                source: variant.source.clone().or_else(|| raw.source.clone()),
            });
        }
    }

    best
}

fn variant_is_active(
    variant: &RawVariant,
    active_betas: &[String],
    active_identity: Option<&str>,
) -> bool {
    let beta_ok = variant
        .requires_beta
        .as_deref()
        .is_none_or(|required| active_betas.iter().any(|beta| beta == required));
    let identity_ok = variant.requires_identity.as_deref().is_none_or(|required| {
        active_identity.is_some_and(|identity| identity.eq_ignore_ascii_case(required))
    });
    beta_ok && identity_ok
}

fn get_case_insensitive<'a, T>(map: &'a BTreeMap<String, T>, key: &str) -> Option<&'a T> {
    map.get(key).or_else(|| {
        map.iter()
            .find(|(candidate, _)| candidate.eq_ignore_ascii_case(key))
            .map(|(_, value)| value)
    })
}

fn parse_toml_value(content: &str) -> Result<Value, toml::de::Error> {
    content.parse::<Value>()
}

fn empty_table() -> Value {
    Value::Table(toml::map::Map::new())
}

fn deep_merge_toml(base: Value, overlay: Value) -> Value {
    match (base, overlay) {
        (Value::Table(mut base_map), Value::Table(overlay_map)) => {
            for (key, overlay_val) in overlay_map {
                let merged = match base_map.remove(&key) {
                    Some(base_val) => deep_merge_toml(base_val, overlay_val),
                    None => overlay_val,
                };
                base_map.insert(key, merged);
            }
            Value::Table(base_map)
        }
        (_, overlay) => overlay,
    }
}

impl ContextCatalog {
    fn from_value(value: Value) -> Result<Self, toml::de::Error> {
        let raw: RawCatalog = value.try_into()?;
        Ok(Self { raw })
    }
}

#[derive(Debug, Clone, Default, Deserialize)]
struct RawCatalog {
    #[serde(default)]
    providers: BTreeMap<String, RawProvider>,
}

#[derive(Debug, Clone, Default, Deserialize)]
struct RawProvider {
    #[serde(default)]
    models: BTreeMap<String, RawModel>,
}

#[derive(Debug, Clone, Default, Deserialize)]
struct RawModel {
    context_window: Option<u64>,
    runtime_context_budget: Option<u64>,
    max_output_tokens: Option<u64>,
    source: Option<String>,
    #[serde(default)]
    variants: BTreeMap<String, RawVariant>,
}

#[derive(Debug, Clone, Default, Deserialize)]
struct RawVariant {
    context_window: Option<u64>,
    runtime_context_budget: Option<u64>,
    max_output_tokens: Option<u64>,
    source: Option<String>,
    requires_beta: Option<String>,
    requires_identity: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bundled_catalog_resolves_codex_contexts() {
        let catalog = ContextCatalog::bundled();

        let sonnet = catalog
            .lookup("openai-codex", "gpt-5.5", &[], None)
            .expect("gpt-5.5 entry");
        let codex = catalog
            .lookup("openai-codex", "gpt-5.3-codex", &[], None)
            .expect("gpt-5.3-codex entry");

        assert_eq!(sonnet.context_window, 1_050_000);
        assert_eq!(sonnet.runtime_context_budget, Some(272_000));
        assert_eq!(codex.context_window, 400_000);
        assert_eq!(codex.runtime_context_budget, Some(272_000));
    }

    #[test]
    fn anthropic_1m_variant_requires_beta() {
        let catalog = ContextCatalog::bundled();

        let base = catalog
            .lookup("anthropic", "claude-sonnet-4-6", &[], None)
            .expect("sonnet entry");
        let beta = catalog
            .lookup(
                "anthropic",
                "claude-sonnet-4-6",
                &["context-1m-2025-08-07".to_string()],
                None,
            )
            .expect("sonnet beta entry");

        assert_eq!(base.context_window, 1_000_000);
        assert_eq!(beta.context_window, 1_000_000);
    }

    #[test]
    fn anthropic_claude_code_variant_uses_one_million_for_opus() {
        let catalog = ContextCatalog::bundled();

        let base = catalog
            .lookup("anthropic", "claude-opus-4-7", &[], None)
            .expect("opus entry");
        let claude_code = catalog
            .lookup("anthropic", "claude-opus-4-7", &[], Some("spoof"))
            .expect("opus claude code entry");

        assert_eq!(base.context_window, 1_000_000);
        assert_eq!(claude_code.context_window, 1_000_000);
    }

    #[test]
    fn workspace_context_catalog_overrides_bundled() {
        let dir = tempfile::tempdir().expect("tempdir");
        let archon_dir = dir.path().join(".archon");
        fs::create_dir_all(&archon_dir).expect("archon dir");
        fs::write(
            archon_dir.join("context.toml"),
            r#"
[providers.openai-codex.models."gpt-5.5"]
context_window = 123456
source = "test"
"#,
        )
        .expect("write context catalog");

        let catalog = ContextCatalog::load(Some(dir.path()));
        let entry = catalog
            .lookup("openai-codex", "gpt-5.5", &[], None)
            .expect("override entry");

        assert_eq!(entry.context_window, 123_456);
        assert_eq!(entry.source.as_deref(), Some("test"));
    }

    #[test]
    fn user_overrides_load_without_bundled_entries() {
        let dir = tempfile::tempdir().expect("tempdir");
        let archon_dir = dir.path().join(".archon");
        fs::create_dir_all(&archon_dir).expect("archon dir");
        fs::write(
            archon_dir.join("context.toml"),
            r#"
[providers.local.models."private-model"]
context_window = 777777
source = "test"
"#,
        )
        .expect("write context catalog");

        let user = ContextCatalog::user_overrides(Some(dir.path()));
        assert!(user.lookup("openai-codex", "gpt-5.5", &[], None).is_none());
        let entry = user
            .lookup("local", "private-model", &[], None)
            .expect("user entry");
        assert_eq!(entry.context_window, 777_777);
    }
}
