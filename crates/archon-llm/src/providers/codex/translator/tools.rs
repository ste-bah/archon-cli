use crate::provider::LlmError;
use crate::providers::codex::types::ResponseTool;

pub fn tools_to_responses_tools(
    tools: &[serde_json::Value],
) -> Result<Vec<ResponseTool>, LlmError> {
    tools
        .iter()
        .map(|tool| {
            let name = tool
                .get("name")
                .and_then(|v| v.as_str())
                .ok_or_else(|| LlmError::Serialize("tool missing name".into()))?
                .to_string();
            let description = tool
                .get("description")
                .and_then(|v| v.as_str())
                .map(str::to_string);
            let parameters = tool
                .get("input_schema")
                .cloned()
                .unwrap_or_else(|| serde_json::json!({"type": "object", "properties": {}}));
            Ok(ResponseTool {
                kind: "function".into(),
                name,
                description,
                parameters,
                strict: None,
            })
        })
        .collect()
}
