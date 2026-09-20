//! Scripted assessment for non-audit integration fixtures. No production fallback.
use serde_json::{Value, json};
pub(crate) fn response(prompt: &str) -> Option<String> {
    let raw = prompt
        .split("## Input\n```json\n")
        .nth(1)?
        .split("\n```")
        .next()?;
    let input: Value = serde_json::from_str(raw).ok()?;
    let contract = input.get("audit_contract")?;
    let root = prompt
        .lines()
        .find_map(|s| s.strip_prefix("repository_root: "))?;
    let records=contract["declared_paths"].as_array()?.iter().map(|p|{
   let exists=std::path::Path::new(root).join(p.as_str().unwrap()).exists();
   json!({"declared_path":p,"verdict":if exists{"exists_as_declared"}else{"absent"},"equivalents":[],
      "required_action":if exists{"none"}else{"deliver"},"reason":"Fixture inventory checked in sealed source; no equivalent declared by this fixture."})
 }).collect::<Vec<_>>();
    Some(json!({"status":"accepted","summary":"fixture source assessed","evidence":[{"kind":"inspection","summary":"fixture source inventory"}],
  "data":{"repository_audit":{"schema_version":1,"snapshot":contract["snapshot"],"records":records}}}).to_string())
}
pub(crate) fn outcome(
    request: &archon_workflow::WorkflowAgentCall,
) -> Option<archon_workflow::WorkflowAgentOutcome> {
    let prompt = request
        .messages
        .iter()
        .filter_map(|m| m["content"].as_str())
        .collect::<Vec<_>>()
        .join("\n");
    Some(archon_workflow::WorkflowAgentOutcome {
        content: response(&prompt)?,
        tool_uses: vec![],
        tokens_in: 0,
        tokens_out: 0,
        stop_reason: Some("end_turn".into()),
    })
}
