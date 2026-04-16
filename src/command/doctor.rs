//! `/doctor` slash command handler.
//! Extracted from main.rs to reduce main.rs from 6234 to < 500 lines.

use std::path::PathBuf;

use crate::slash_context::SlashCommandContext;
use archon_tui::app::TuiEvent;

/// Handle `/doctor` — diagnostic status display.
pub async fn handle_doctor_command(
    tui_tx: &tokio::sync::mpsc::Sender<TuiEvent>,
    ctx: &SlashCommandContext,
) {
    use archon_core::env_vars;

    let mut out = String::from("\nArchon diagnostics:\n");

    // Auth
    out.push_str(&format!("  Auth: authenticated ({})\n", ctx.auth_label));

    // MCP servers
    let states = ctx.mcp_manager.get_server_states().await;
    if states.is_empty() {
        out.push_str("  MCP servers: none configured\n");
    } else {
        out.push_str(&format!("  MCP servers: {} configured\n", states.len()));
        for (name, state) in &states {
            out.push_str(&format!("    {name}: {state}\n"));
        }
    }

    // Memory graph
    match ctx.memory.memory_count() {
        Ok(count) => out.push_str(&format!("  Memory graph: open ({count} memories)\n")),
        Err(e) => out.push_str(&format!("  Memory graph: error ({e})\n")),
    }

    // Config
    let config_valid = ctx.config_path.exists();
    out.push_str(&format!(
        "  Config: {} ({})\n",
        ctx.config_path.display(),
        if config_valid { "valid" } else { "not found" },
    ));

    // Checkpoint store
    let ckpt_path = dirs::data_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("archon")
        .join("checkpoints.db");
    let ckpt_status = if ckpt_path.exists() { "open" } else { "closed" };
    out.push_str(&format!("  Checkpoint store: {ckpt_status}\n"));

    // Model
    let current_model = {
        let ov = ctx.model_override_shared.lock().await;
        if ov.is_empty() {
            ctx.default_model.clone()
        } else {
            ov.clone()
        }
    };
    out.push_str(&format!("  Model: {current_model}\n"));

    // Environment variables
    out.push_str(&env_vars::format_doctor_env_vars(&ctx.env_vars));

    let _ = tui_tx.send(TuiEvent::TextDelta(out)).await;
}
