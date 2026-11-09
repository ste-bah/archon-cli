use anyhow::{Result, anyhow};
use archon_workflow::prd_tools::{
    PrdToolDefect, actionable_tokens, prd_tool_declaration, prd_tool_defects,
};
use std::collections::BTreeMap;
use std::path::Path;

pub(crate) fn validate_prd_launch_input(
    prd_path: &Path,
    project_root: &Path,
) -> Result<(Vec<u8>, String, BTreeMap<String, String>)> {
    let input = super::workflow_task_set::validate_prd_input(prd_path)?;
    let (config, permitted) = super::workflow_mcp::project_permitted_tools(project_root)
        .map_err(|error| anyhow!(error))?;
    let text = std::str::from_utf8(&input.0)?;
    let actionable = actionable_tokens(text);
    let messages: Vec<_> = prd_tool_defects(&prd_tool_declaration(text), &permitted).iter()
        .filter(|defect| matches!(defect, PrdToolDefect::NotPermitted { token } if actionable.contains(token)))
        .map(|defect| defect.message(prd_path, &config)).collect();
    if !messages.is_empty() {
        return Err(anyhow!(messages.join(" | ")));
    }
    Ok(input)
}
