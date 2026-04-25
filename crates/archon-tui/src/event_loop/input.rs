//! Keyboard / mouse / resize `crossterm` event dispatch extracted from
//! `run_inner`.
//!
//! Relocated from `src/event_loop.rs` (L372-L648 inclusive — the full
//! `match event::read()? { ... }` body) per REM-2g (split plan section
//! 3.3, docs/rem-2-split-plan.md).
//!
//! Behavioral equivalence note: the original block is a single `match` on
//! `Event::Mouse | Event::Key | Event::Resize | _` running inside the outer
//! `loop { ... }` of `run_inner`. Inside `Event::Key`, overlay branches
//! terminate with `continue;` to re-enter the outer loop. In the extracted
//! `handle_key_event` function, `continue;` is converted to `return;` —
//! semantically identical because the caller's next action is always the
//! outer `loop` top (render + poll). No branches of the original rely on
//! `continue` advancing internal state other than via `&mut app`.
//!
//! The `#[allow(clippy::cognitive_complexity)]` on the original `run_inner`
//! is replicated here because this function inherits the keyboard-branch
//! complexity.

use crossterm::event::{Event, KeyCode, KeyModifiers, MouseEventKind};

use crate::app::{App, McpManagerView, should_process_key_event};
use crate::vim::{VimAction, VimState};

use super::{mcp_action_count, mcp_actions_for};

/// Handle a single `crossterm::Event` against the running `App`.
///
/// Caller is responsible for checking `event::poll` beforehand; this
/// function assumes an event is already available and has been read.
#[allow(clippy::cognitive_complexity)]
pub(super) async fn handle_key_event(
    app: &mut App,
    event: Event,
    input_tx: &tokio::sync::mpsc::Sender<String>,
    btw_tx: Option<&tokio::sync::mpsc::Sender<String>>,
    permission_tx: Option<&tokio::sync::mpsc::Sender<bool>>,
    keymap: &crate::keybindings::KeyMap,
) {
    match event {
        Event::Mouse(mouse) => match mouse.kind {
            MouseEventKind::ScrollUp => {
                app.output.scroll_up(3);
            }
            MouseEventKind::ScrollDown => {
                app.output.scroll_down(3);
            }
            _ => {}
        },
        Event::Key(key) => {
            // Windows emits both Press and Release for each keystroke;
            // process only Press and Repeat to avoid double input.
            if !should_process_key_event(&key) {
                return;
            }
            // Handle session picker — Up/Down/Enter/Esc
            if app.session_picker.is_some() {
                match key.code {
                    KeyCode::Up => {
                        if let Some(ref mut picker) = app.session_picker {
                            if picker.selected > 0 {
                                picker.selected -= 1;
                            } else {
                                picker.selected = picker.sessions.len().saturating_sub(1);
                            }
                        }
                        return;
                    }
                    KeyCode::Down => {
                        if let Some(ref mut picker) = app.session_picker {
                            if picker.selected + 1 < picker.sessions.len() {
                                picker.selected += 1;
                            } else {
                                picker.selected = 0;
                            }
                        }
                        return;
                    }
                    KeyCode::Enter => {
                        if let Some(picker) = app.session_picker.take()
                            && let Some(s) = picker.sessions.get(picker.selected)
                        {
                            let _ = input_tx.send(format!("__resume_session__ {}", s.id)).await;
                        }
                        return;
                    }
                    KeyCode::Esc => {
                        app.session_picker = None;
                        return;
                    }
                    _ => return, // swallow other keys
                }
            }
            // Handle MCP manager overlay — Up/Down/Enter/Esc
            if app.mcp_manager.is_some() {
                match key.code {
                    KeyCode::Up => {
                        if let Some(ref mut mgr) = app.mcp_manager {
                            match &mut mgr.view {
                                McpManagerView::ServerList { selected } => {
                                    if *selected > 0 {
                                        *selected -= 1;
                                    } else {
                                        *selected = mgr.servers.len().saturating_sub(1);
                                    }
                                }
                                McpManagerView::ServerMenu {
                                    action_idx,
                                    server_idx,
                                } => {
                                    let count = mcp_action_count(mgr.servers.get(*server_idx));
                                    if *action_idx > 0 {
                                        *action_idx -= 1;
                                    } else {
                                        *action_idx = count.saturating_sub(1);
                                    }
                                }
                                McpManagerView::ToolList { scroll, .. } => {
                                    *scroll = scroll.saturating_sub(1);
                                }
                            }
                        }
                        return;
                    }
                    KeyCode::Down => {
                        if let Some(ref mut mgr) = app.mcp_manager {
                            match &mut mgr.view {
                                McpManagerView::ServerList { selected } => {
                                    if *selected + 1 < mgr.servers.len() {
                                        *selected += 1;
                                    } else {
                                        *selected = 0;
                                    }
                                }
                                McpManagerView::ServerMenu {
                                    action_idx,
                                    server_idx,
                                } => {
                                    let action_count =
                                        mcp_action_count(mgr.servers.get(*server_idx));
                                    if *action_idx + 1 < action_count {
                                        *action_idx += 1;
                                    } else {
                                        *action_idx = 0;
                                    }
                                }
                                McpManagerView::ToolList { scroll, tools, .. } => {
                                    if *scroll + 1 < tools.len() {
                                        *scroll += 1;
                                    }
                                }
                            }
                        }
                        return;
                    }
                    KeyCode::Enter => {
                        if let Some(ref mut mgr) = app.mcp_manager {
                            match mgr.view.clone() {
                                McpManagerView::ServerList { selected } => {
                                    if !mgr.servers.is_empty() {
                                        mgr.view = McpManagerView::ServerMenu {
                                            server_idx: selected,
                                            action_idx: 0,
                                        };
                                    }
                                }
                                McpManagerView::ServerMenu {
                                    server_idx,
                                    action_idx,
                                } => {
                                    if let Some(server) = mgr.servers.get(server_idx) {
                                        let actions = mcp_actions_for(server);
                                        if let Some(action) = actions.get(action_idx) {
                                            match *action {
                                                "back" => {
                                                    mgr.view = McpManagerView::ServerList {
                                                        selected: server_idx,
                                                    };
                                                }
                                                "tools" => {
                                                    mgr.view = McpManagerView::ToolList {
                                                        server_name: server.name.clone(),
                                                        tools: server.tools.clone(),
                                                        scroll: 0,
                                                    };
                                                }
                                                _ => {
                                                    let cmd = format!(
                                                        "__mcp_action__ {} {}",
                                                        server.name, action
                                                    );
                                                    let _ = input_tx.send(cmd).await;
                                                }
                                            }
                                        }
                                    }
                                }
                                McpManagerView::ToolList { .. } => {
                                    // Enter/Esc handled below — nothing to do on Enter
                                }
                            }
                        }
                        return;
                    }
                    KeyCode::Esc => {
                        if let Some(ref mut mgr) = app.mcp_manager {
                            match &mgr.view {
                                McpManagerView::ToolList { server_name, .. } => {
                                    // Find the server index to return to its menu
                                    let idx = mgr
                                        .servers
                                        .iter()
                                        .position(|s| s.name == *server_name)
                                        .unwrap_or(0);
                                    mgr.view = McpManagerView::ServerMenu {
                                        server_idx: idx,
                                        action_idx: 0,
                                    };
                                }
                                McpManagerView::ServerMenu { server_idx, .. } => {
                                    let idx = *server_idx;
                                    mgr.view = McpManagerView::ServerList { selected: idx };
                                }
                                McpManagerView::ServerList { .. } => {
                                    app.mcp_manager = None;
                                }
                            }
                        }
                        return;
                    }
                    _ => return, // swallow other keys while overlay is up
                }
            }
            // Handle permission prompt — y/n/Enter/Esc
            if app.permission_prompt.is_some() {
                match key.code {
                    KeyCode::Char('y') | KeyCode::Char('Y') | KeyCode::Enter => {
                        let tool = app.permission_prompt.take().unwrap_or_default();
                        if let Some(tx) = permission_tx {
                            let _ = tx.send(true).await;
                        }
                        app.output.append_line(&format!("[{tool}: approved]"));
                        return;
                    }
                    KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => {
                        let tool = app.permission_prompt.take().unwrap_or_default();
                        if let Some(tx) = permission_tx {
                            let _ = tx.send(false).await;
                        }
                        app.output.append_line(&format!("[{tool}: denied]"));
                        return;
                    }
                    _ => return, // swallow other keys during permission prompt
                }
            }
            // Dismiss /btw overlay on any of Esc/Enter/Space
            if app.btw_overlay.is_some() {
                match key.code {
                    KeyCode::Esc | KeyCode::Enter | KeyCode::Char(' ') => {
                        app.btw_overlay = None;
                        return;
                    }
                    _ => return, // swallow all other keys while overlay is up
                }
            }
            // Handle message selector overlay — Up/Down/Enter/Esc
            // (TASK-TUI-620-followup). Mirrors the session_picker branch
            // above: Enter emits a protocol message that the session.rs
            // consumer turns into a SessionStore truncation.
            if app.message_selector.is_some() {
                match key.code {
                    KeyCode::Up => {
                        if let Some(ref mut sel) = app.message_selector {
                            sel.select_prev();
                        }
                        return;
                    }
                    KeyCode::Down => {
                        if let Some(ref mut sel) = app.message_selector {
                            sel.select_next();
                        }
                        return;
                    }
                    KeyCode::Enter => {
                        if let Some(sel) = app.message_selector.take() {
                            let idx = sel.selected_index;
                            let _ = input_tx.send(format!("__truncate_session__ {}", idx)).await;
                        }
                        return;
                    }
                    KeyCode::Esc => {
                        app.message_selector = None;
                        return;
                    }
                    _ => return, // swallow other keys while overlay is up
                }
            }
            // Handle skills menu overlay — Up/Down/Enter/Esc
            // (TASK-TUI-627-followup). Enter injects `/{skill-name} `
            // into the input buffer via InputHandler::set_text, then
            // closes the overlay so the user can finish typing args.
            if app.skills_menu.is_some() {
                match key.code {
                    KeyCode::Up => {
                        if let Some(ref mut menu) = app.skills_menu {
                            menu.select_prev();
                        }
                        return;
                    }
                    KeyCode::Down => {
                        if let Some(ref mut menu) = app.skills_menu {
                            menu.select_next();
                        }
                        return;
                    }
                    KeyCode::Enter => {
                        if let Some(menu) = app.skills_menu.take() {
                            if let Some(skill) = menu.selected() {
                                app.input.set_text(&format!("/{} ", skill.name));
                            }
                        }
                        return;
                    }
                    KeyCode::Esc => {
                        app.skills_menu = None;
                        return;
                    }
                    _ => return, // swallow other keys while overlay is up
                }
            }
            // Vim mode key routing — Ctrl+D / Ctrl+C fall through to normal handling
            let is_ctrl_quit = key.modifiers == KeyModifiers::CONTROL
                && matches!(key.code, KeyCode::Char('d') | KeyCode::Char('c'));
            if !is_ctrl_quit && let Some(ref mut vim) = app.vim_state {
                let action = vim.handle_key(key);
                match action {
                    VimAction::Submit => {
                        let text = vim.text();
                        *vim = VimState::new();
                        if !text.trim().is_empty() {
                            if app.is_generating {
                                app.pending_input.push(text);
                                app.output
                                    .append_line("[queued — will send after current turn]");
                            } else {
                                let _ = input_tx.send(text).await;
                            }
                        }
                    }
                    VimAction::Quit => {
                        app.vim_state = None;
                    }
                    _ => {}
                }
                return;
            }
            match crate::input::handle_key(app, key, keymap) {
                crate::input::KeyResult::Nothing => {}
                crate::input::KeyResult::Quit => {
                    app.should_quit = true;
                }
                crate::input::KeyResult::SendInput(text) => {
                    let _ = input_tx.send(text).await;
                }
                crate::input::KeyResult::SendCancel => {
                    let _ = input_tx.try_send("__cancel__".to_string());
                }
                crate::input::KeyResult::SendBtw(q) => {
                    if let Some(btw) = btw_tx {
                        let _ = btw.send(q).await;
                    }
                }
            }
        }
        Event::Resize(cols, rows) => {
            crate::layout::handle_resize(cols, rows);
        }
        _ => {} // FocusGained/FocusLost/Paste
    }
}
