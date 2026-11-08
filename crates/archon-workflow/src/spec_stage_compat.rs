//! Preserve historical stage inputs before validating authored requirements.
use super::spec::{ProviderTier, ReducerKind, RetryPolicy, StageKind, StageSpec};
use serde::Deserialize;
use std::collections::BTreeMap;

#[derive(Deserialize)]
struct StageSpecDe {
    id: String,
    kind: StageKind,
    #[serde(default)]
    task: Option<String>,
    #[serde(default)]
    agent: Option<String>,
    #[serde(default)]
    foreach: Option<String>,
    #[serde(default)]
    reducer: Option<ReducerKind>,
    #[serde(default)]
    tool: Option<String>,
    #[serde(default)]
    depends_on: Vec<String>,
    #[serde(default)]
    provider_tier: Option<ProviderTier>,
    #[serde(default)]
    retry: RetryPolicy,
    #[serde(default)]
    input: serde_json::Value,
    #[serde(default)]
    model: Option<String>,
    #[serde(default)]
    provider: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    expected_target_files: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    verify_command: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    max_parallelism: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    item_kind: Option<StageKind>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    filter: Option<String>,
    /// Additional stage declarations are forwarded with the typed tool requirements.
    #[serde(default, flatten, skip_serializing_if = "BTreeMap::is_empty")]
    extra: BTreeMap<String, serde_json::Value>,
}

impl<'de> Deserialize<'de> for StageSpec {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let mut stage = StageSpecDe::deserialize(deserializer)?;
        let tools = stage.extra.remove("required_tools");
        let required_tools = tools.as_ref().and_then(string_or_list).unwrap_or_default();
        if required_tools.is_empty()
            && let Some(tools) = tools
        {
            stage.extra.insert("required_tools".into(), tools);
        }
        Ok(Self {
            id: stage.id,
            kind: stage.kind,
            task: stage.task,
            agent: stage.agent,
            foreach: stage.foreach,
            reducer: stage.reducer,
            tool: stage.tool,
            depends_on: stage.depends_on,
            provider_tier: stage.provider_tier,
            retry: stage.retry,
            input: stage.input,
            model: stage.model,
            provider: stage.provider,
            expected_target_files: stage.expected_target_files,
            verify_command: stage.verify_command,
            max_parallelism: stage.max_parallelism,
            item_kind: stage.item_kind,
            filter: stage.filter,
            extra: stage.extra,
            required_tools,
        })
    }
}

fn string_or_list(value: &serde_json::Value) -> Option<Vec<String>> {
    match value {
        serde_json::Value::Null => Some(Vec::new()),
        serde_json::Value::String(value) => Some(vec![value.clone()]),
        serde_json::Value::Array(values) => values
            .iter()
            .map(|value| value.as_str().map(str::to_owned))
            .collect(),
        _ => None,
    }
}

pub(crate) fn validate_tools(stage: &StageSpec) -> crate::error::WorkflowResult<()> {
    if let Some(value) = stage.extra.get("required_tools") {
        let invalid = match value {
            serde_json::Value::Null => None,
            serde_json::Value::Array(values) => values.iter().find(|value| !value.is_string()),
            value => Some(value),
        };
        if let Some(entry) = invalid {
            return Err(crate::error::WorkflowError::SpecInvalid(format!(
                "stage '{}' required_tools entry '{entry}' is not a string",
                stage.id
            )));
        }
    }
    Ok(())
}
