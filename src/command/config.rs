//! `/config` slash command handler.
//! Extracted from main.rs to reduce main.rs from 6234 to < 500 lines.

use crate::cli_args::Cli;
use crate::slash_context::SlashCommandContext;
use archon_tui::app::TuiEvent;

/// Handle `/config` commands: list, get, set.
pub async fn handle_config_command(
    input: &str,
    tui_tx: &tokio::sync::mpsc::Sender<TuiEvent>,
    ctx: &SlashCommandContext,
) {
    let args: Vec<&str> = input
        .strip_prefix("/config")
        .unwrap_or_default()
        .trim()
        .splitn(2, ' ')
        .collect();
    let key = args.first().map(|s| s.trim()).unwrap_or("");
    let value = args.get(1).map(|s| s.trim()).unwrap_or("");

    if key == "sources" {
        let output = archon_core::config_source::format_sources(&ctx.config_sources);
        if output.is_empty() {
            let _ = tui_tx
                .send(TuiEvent::TextDelta("\nNo config sources tracked.\n".into()))
                .await;
        } else {
            let _ = tui_tx
                .send(TuiEvent::TextDelta(format!("\nConfig sources:\n{output}")))
                .await;
        }
        return;
    }

    if key.is_empty() {
        // List all config keys with current values
        let keys = archon_tools::config_tool::all_keys();
        let mut lines = String::from("\nRuntime configuration:\n");
        for k in &keys {
            let val =
                archon_tools::config_tool::get_config_value(k).unwrap_or_else(|| "(unknown)".into());
            lines.push_str(&format!("  {k} = {val}\n"));
        }
        let _ = tui_tx.send(TuiEvent::TextDelta(lines)).await;
    } else if value.is_empty() {
        // Get a single key
        match archon_tools::config_tool::get_config_value(key) {
            Some(val) => {
                let _ = tui_tx
                    .send(TuiEvent::TextDelta(format!("\n{key} = {val}\n")))
                    .await;
            }
            None => {
                let _ = tui_tx
                    .send(TuiEvent::Error(format!("Unknown config key: {key}")))
                    .await;
            }
        }
    } else {
        // Set key=value via the ConfigTool
        use archon_tools::tool::{AgentMode, ToolContext};
        let tool = archon_tools::config_tool::ConfigTool;
        let tool_ctx = ToolContext {
            working_dir: std::env::current_dir().unwrap_or_default(),
            session_id: String::new(),
            mode: AgentMode::Normal,
            extra_dirs: Vec::new(),
            ..Default::default()
        };
        let result = archon_tools::tool::Tool::execute(
            &tool,
            serde_json::json!({ "action": "set", "key": key, "value": value }),
            &tool_ctx,
        )
        .await;
        if result.is_error {
            let _ = tui_tx.send(TuiEvent::Error(result.content)).await;
        } else {
            let _ = tui_tx
                .send(TuiEvent::TextDelta(format!("\n{}\n", result.content)))
                .await;
        }
    }
}
