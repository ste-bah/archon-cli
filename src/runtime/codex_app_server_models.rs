//! Codex app-server model discovery and fallback catalog handling.

use std::sync::{Arc, RwLock};

use archon_core::config::CodexProviderConfig;
use archon_llm::provider::ModelInfo;

use super::codex_app_server_rpc::CodexAppServerRpcClient;

pub(crate) type ModelCache = Arc<RwLock<Vec<ModelInfo>>>;

pub(crate) fn fallback_models(config: &CodexProviderConfig) -> Vec<ModelInfo> {
    config
        .app_server_model_catalog
        .iter()
        .map(|id| model_info(id, id))
        .collect()
}

pub(crate) async fn refresh_model_cache(
    client: &CodexAppServerRpcClient,
    timeout_ms: u64,
    cache: &ModelCache,
) {
    let Ok(response) = client
        .request("model/list", serde_json::json!({}), timeout_ms)
        .await
    else {
        return;
    };
    let models = parse_model_list(&response);
    if models.is_empty() {
        return;
    }
    if let Ok(mut guard) = cache.write() {
        *guard = models;
    }
}

fn parse_model_list(response: &serde_json::Value) -> Vec<ModelInfo> {
    let Some(items) = response.get("data").and_then(serde_json::Value::as_array) else {
        return Vec::new();
    };
    items
        .iter()
        .filter(|item| {
            !item
                .get("hidden")
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(false)
        })
        .filter_map(parse_model)
        .collect()
}

fn parse_model(item: &serde_json::Value) -> Option<ModelInfo> {
    let id = read_string(item, "id").or_else(|| read_string(item, "model"))?;
    let display = read_string(item, "displayName").unwrap_or(id);
    Some(model_info(id, display))
}

fn model_info(id: &str, display_name: &str) -> ModelInfo {
    ModelInfo {
        id: id.to_string(),
        display_name: display_name.to_string(),
        context_window: 200_000,
    }
}

fn read_string<'a>(value: &'a serde_json::Value, key: &str) -> Option<&'a str> {
    value
        .get(key)
        .and_then(serde_json::Value::as_str)
        .filter(|text| !text.trim().is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_model_list_and_filters_hidden_models() {
        let response = serde_json::json!({
            "data": [
                {"id": "gpt-5.5", "displayName": "GPT 5.5", "hidden": false},
                {"model": "gpt-hidden", "hidden": true},
                {"model": "gpt-5.4-mini"}
            ]
        });

        let models = parse_model_list(&response);

        assert_eq!(models.len(), 2);
        assert_eq!(models[0].id, "gpt-5.5");
        assert_eq!(models[0].display_name, "GPT 5.5");
        assert_eq!(models[1].id, "gpt-5.4-mini");
    }

    #[test]
    fn fallback_models_use_configured_catalog() {
        let config = CodexProviderConfig {
            app_server_model_catalog: vec!["one".into(), "two".into()],
            ..CodexProviderConfig::default()
        };

        let models = fallback_models(&config);

        assert_eq!(
            models.iter().map(|m| m.id.as_str()).collect::<Vec<_>>(),
            vec!["one", "two"]
        );
    }
}
