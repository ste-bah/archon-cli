use std::sync::Arc;

use archon_core::agent::Agent;

use super::mcp_task::{self, McpLifecycleTx};
use super::session_history::{handle_resume_session, handle_truncate_session};
use crate::slash_context::SlashCommandContext;

pub(super) struct ControlInputContext<'a> {
    pub(super) agent: &'a Arc<tokio::sync::Mutex<Agent>>,
    pub(super) input_tui_tx: &'a archon_tui::event_channel::TuiEventSender,
    pub(super) session_store: &'a Arc<archon_session::storage::SessionStore>,
    pub(super) session_id: &'a str,
    pub(super) adapter: &'a Arc<crate::agent_handle::AgentHandle>,
    pub(super) dispatcher: &'a Arc<std::sync::Mutex<archon_tui::AgentDispatcher>>,
    pub(super) mcp_lifecycle_tx: &'a McpLifecycleTx,
    pub(super) cmd_ctx: &'a SlashCommandContext,
    pub(super) active_session: &'a crate::session::active_session::ActiveSessionId,
}

pub(super) async fn handle_control_input(input: &str, ctx: ControlInputContext<'_>) -> bool {
    if let Some(session_id) = input.strip_prefix("__resume_session__ ") {
        let session_id = session_id.trim();
        let resumed =
            handle_resume_session(ctx.agent, ctx.input_tui_tx, ctx.session_store, session_id).await;
        // Continue the session that was just restored instead of the row this
        // process created at launch.
        //
        // Without this, resuming forks: the restored conversation is written
        // back under the launch id by `post_turn`, the resumed row stays frozen
        // at whatever it last held, and cost lands in a third place. Observed
        // directly -- resuming a 2-turn session and running one turn left that
        // row at 2 turns and moved all six messages into the launch row.
        //
        // Only on success. A failed load leaves the agent holding the previous
        // conversation, and repointing writes at a session we could not read
        // would overwrite it with content that does not belong to it.
        if resumed {
            ctx.active_session.set(session_id);
        }
        return true;
    }
    if let Some(idx_str) = input.strip_prefix("__truncate_session__ ") {
        handle_truncate_session(
            ctx.agent,
            ctx.input_tui_tx,
            ctx.session_store,
            ctx.session_id,
            idx_str.trim(),
        )
        .await;
        return true;
    }
    if input == "__cancel__" {
        cancel_inflight_turn(ctx.adapter, ctx.dispatcher, ctx.session_id);
        return true;
    }
    if let Some(rest) = input.strip_prefix("__mcp_action__ ") {
        mcp_task::handle_overlay_action(
            rest,
            ctx.mcp_lifecycle_tx,
            &ctx.cmd_ctx.mcp_manager,
            ctx.input_tui_tx,
        )
        .await;
        return true;
    }
    false
}

fn cancel_inflight_turn(
    adapter: &Arc<crate::agent_handle::AgentHandle>,
    dispatcher: &Arc<std::sync::Mutex<archon_tui::AgentDispatcher>>,
    session_id: &str,
) {
    adapter.fire_cancel();
    // Release whatever the aborted turn was holding in topology admission.
    //
    // Claims are taken at admission and released by `on_tool_run_outcome`,
    // which a cancelled attempt never reaches. Without this, a spawn's
    // live-agent slot and a write's path claims survive the turn that took
    // them, and every later write in the session is refused as conflicting
    // with a claim nothing holds -- so one Ctrl+C breaks the session until
    // restart. Cancelling is normal operation, not an edge case, so this runs
    // unconditionally and is a no-op when admission is disabled or the session
    // is untracked.
    crate::command::topology_admission::reset_session(session_id);
    match dispatcher.lock().unwrap().cancel_current() {
        archon_tui::CancelOutcome::NoInflight => {
            tracing::debug!("Ctrl+C: no in-flight turn to cancel");
        }
        archon_tui::CancelOutcome::Aborted { elapsed_ms } => {
            tracing::info!(
                elapsed_ms,
                "Ctrl+C: aborted in-flight turn; released topology admission claims"
            );
        }
    }
}
