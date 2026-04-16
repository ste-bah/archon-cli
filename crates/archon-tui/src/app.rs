use std::io;

use crossterm::ExecutableCommand;
use crossterm::event::{
    self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEvent, KeyEventKind,
    KeyModifiers, MouseEventKind,
};
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{
    Block, Borders, List, ListItem, Paragraph, Scrollbar, ScrollbarOrientation, ScrollbarState,
    Wrap,
};

use crate::input::InputHandler;
use crate::markdown::render_markdown_line;
use crate::output::{OutputBuffer, ThinkingState, ToolOutputState};
use crate::splash::{self, ActivityEntry};
use crate::split_pane::SplitPaneManager;
use crate::status::StatusBar;
use crate::terminal::TerminalGuard;
use crate::theme::{Theme, intj_theme};
use crate::ultrathink;
use crate::vim::{VimAction, VimState};

/// Message from the agent loop to the TUI.
#[derive(Debug, Clone)]
pub enum TuiEvent {
    TextDelta(String),
    ThinkingDelta(String),
    ToolStart {
        name: String,
        id: String,
    },
    ToolComplete {
        name: String,
        id: String,
        success: bool,
        output: String,
    },
    TurnComplete {
        input_tokens: u64,
        output_tokens: u64,
    },
    Error(String),
    /// Sent by main.rs right before agent.process_message(). This is the ONLY
    /// place is_generating should be set to true — at the point generation
    /// actually starts, not at input submission time.
    GenerationStarted,
    /// Sent by main.rs after a slash command is handled. Resets is_generating
    /// in case it was set by a prior event.
    SlashCommandComplete,
    /// Toggle thinking display on/off in the TUI.
    ThinkingToggle(bool),
    /// Update the model name shown in the status bar.
    ModelChanged(String),
    /// /btw side question response — show as overlay.
    BtwResponse(String),
    /// Permission prompt — agent wants to use a risky tool, needs y/n.
    PermissionPrompt {
        tool: String,
        description: String,
    },
    /// Session was renamed — show name badge on input line.
    SessionRenamed(String),
    /// Permission mode changed — update status bar and permission indicator.
    PermissionModeChanged(String),
    /// Show interactive session picker for /resume.
    ShowSessionPicker(Vec<SessionPickerEntry>),
    /// Set the accent color on the active theme (used by /color).
    SetAccentColor(ratatui::style::Color),
    /// Replace the entire theme by name (used by /theme).
    SetTheme(String),
    /// Show MCP server manager overlay.
    ShowMcpManager(Vec<McpServerEntry>),
    /// Update MCP server manager with fresh state (after reconnect/disable).
    UpdateMcpManager(Vec<McpServerEntry>),
    /// Enable or disable vim keybindings (from config at startup).
    SetVimMode(bool),
    /// Toggle vim keybindings on/off (used by /vim slash command).
    VimToggle,
    /// Transcribed voice text — inject into the input buffer.
    VoiceText(String),
    /// Set the active agent name and color in the status bar (AGT-015).
    SetAgentInfo {
        name: String,
        color: Option<String>,
    },
    /// Terminal was resized — route through `crate::layout::handle_resize`
    /// to record the new dimensions and mark the next frame dirty (TUI-105).
    Resize {
        cols: u16,
        rows: u16,
    },
    /// User submitted a prompt via the input line. Consumed by
    /// `run_event_loop` (TUI-106).
    UserInput(String),
    /// User pressed /cancel — the dispatcher should abort the in-flight
    /// turn. Consumed by `run_event_loop` (TUI-106).
    SlashCancel,
    /// User ran /agent <id> — the dispatcher should switch the active
    /// agent. Consumed by `run_event_loop` (TUI-106).
    SlashAgent(String),
    Done,
}

/// Callback type for sending user input to the agent loop.
pub type InputSender = tokio::sync::mpsc::Sender<String>;

/// The main TUI application state.
pub struct App {
    pub output: OutputBuffer,
    pub input: InputHandler,
    pub status: StatusBar,
    pub thinking: ThinkingState,
    pub theme: Theme,
    pub should_quit: bool,
    pub is_generating: bool,
    /// Currently running tool name (shown in status bar, not output).
    pub active_tool: Option<String>,
    /// Collapsible tool output blocks for the current turn.
    pub tool_outputs: Vec<ToolOutputState>,
    /// Whether to display thinking text (toggle with /thinking).
    pub show_thinking: bool,
    /// Timestamp of last Esc press for double-Esc cancel detection.
    last_esc: Option<std::time::Instant>,
    /// Show the splash screen until the first user input.
    pub show_splash: bool,
    /// Model name displayed on the splash screen.
    pub splash_model: String,
    /// Working directory displayed on the splash screen.
    pub splash_working_dir: String,
    /// Recent activity entries for the splash screen.
    pub splash_activity: Vec<ActivityEntry>,
    /// Input queued while the agent was generating (sent after turn completes).
    pub pending_input: Vec<String>,
    /// /btw side question overlay — shown on top of the main output.
    /// Dismissed with Esc/Enter/Space.
    pub btw_overlay: Option<String>,
    /// Pending permission prompt — tool name waiting for y/n.
    pub permission_prompt: Option<String>,
    /// Session name (shown right-aligned on input line after /rename).
    pub session_name: Option<String>,
    /// Active session picker modal (shown by /resume).
    pub session_picker: Option<SessionPicker>,
    /// Active MCP server manager modal (shown by /mcp).
    pub mcp_manager: Option<McpManager>,
    /// Vim keybinding state — Some when vim mode is active, None otherwise.
    pub vim_state: Option<VimState>,
    /// Split pane layout and state manager.
    pub panes: SplitPaneManager,
}

impl Default for App {
    fn default() -> Self {
        Self {
            output: OutputBuffer::new(),
            input: InputHandler::new(),
            status: StatusBar::default(),
            thinking: ThinkingState::new(),
            theme: intj_theme(),
            should_quit: false,
            is_generating: false,
            active_tool: None,
            tool_outputs: Vec::new(),
            show_thinking: false,
            last_esc: None,
            show_splash: true,
            splash_model: String::from("claude-sonnet-4-6"),
            splash_working_dir: String::new(),
            splash_activity: Vec::new(),
            pending_input: Vec::new(),
            btw_overlay: None,
            permission_prompt: None,
            session_name: None,
            session_picker: None,
            mcp_manager: None,
            vim_state: None,
            panes: SplitPaneManager::new(),
        }
    }
}

impl App {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn on_text_delta(&mut self, text: &str) {
        // A non-thinking event while thinking is active means thinking ended.
        if self.thinking.active {
            self.finish_thinking();
        }
        self.output.append(text);
    }

    pub fn on_thinking_delta(&mut self, text: &str) {
        // Always track timing (for accurate "Thought for Xs" display).
        // Only accumulate text when show_thinking is on.
        if !self.thinking.active {
            self.thinking.active = true;
            self.thinking.start = Some(std::time::Instant::now());
        }
        if self.show_thinking {
            self.thinking.accumulated.push_str(text);
        }
    }

    pub fn on_tool_start(&mut self, name: &str, id: &str) {
        if self.thinking.active {
            self.finish_thinking();
        }
        // Track active tool for status bar, but don't clutter the output.
        // is_generating is already set by GenerationStarted — not set here.
        self.active_tool = Some(name.to_string());
        self.tool_outputs.push(ToolOutputState::new(name, id));
    }

    pub fn on_tool_complete(&mut self, name: &str, id: &str, success: bool, output: &str) {
        // Only clear active_tool if it matches the completing tool (guards against overlapping calls)
        if self.active_tool.as_deref() == Some(name) {
            self.active_tool = None;
        }
        // Find the matching tool output and mark complete
        if let Some(tool_state) = self.tool_outputs.iter_mut().rev().find(|t| t.tool_id == id) {
            tool_state.complete(output, !success);
        }
        if !success {
            // Only show tool failures — they're actionable information
            self.output.append_line(&format!("[tool] {name} failed"));
        }
    }

    /// Toggle expand/collapse on the last tool output, or a specific one by index.
    pub fn toggle_tool_output(&mut self, index: Option<usize>) {
        if let Some(idx) = index {
            if let Some(tool) = self.tool_outputs.get_mut(idx) {
                tool.toggle_expand();
            }
        } else if let Some(tool) = self.tool_outputs.last_mut() {
            tool.toggle_expand();
        }
    }

    pub fn on_turn_complete(&mut self) {
        if self.thinking.active {
            self.finish_thinking();
        }
        self.is_generating = false;
        self.output.append_line("");
        // Reset thinking for the next turn.
        self.thinking.reset();
    }

    pub fn on_error(&mut self, message: &str) {
        if self.thinking.active {
            self.finish_thinking();
        }
        self.output.append_line(&format!("[error] {message}"));
        self.is_generating = false;
    }

    pub fn submit_input(&mut self) -> String {
        let text = self.input.submit();
        if !text.is_empty() {
            self.show_splash = false;
            // Auto-scroll to bottom so the user sees their prompt and response
            self.output.scroll_to_bottom();
            self.output.append_line(&format!("> {text}"));
        }
        text
    }

    pub fn on_generation_started(&mut self) {
        self.is_generating = true;
    }

    pub fn on_slash_command_complete(&mut self) {
        self.is_generating = false;
    }

    /// Finalize the current thinking block. The summary is rendered as a
    /// separate indicator by `thinking_lines()` — nothing is appended to
    /// the output buffer so we avoid cluttering tool output with repeated
    /// "+ Thought for 0ms" lines.
    fn finish_thinking(&mut self) {
        self.thinking.on_thinking_complete();
    }

    // -- rendering helpers --------------------------------------------------

    /// Build the `Line`s for the thinking indicator (inserted into the output
    /// area at the bottom, before the cursor).
    pub fn thinking_lines(&self) -> Vec<Line<'_>> {
        let t = &self.theme;
        if self.thinking.active {
            if self.thinking.expanded {
                // Expanded: show full text in dim italic
                let mut lines = vec![Line::from(Span::styled(
                    "- Thinking:",
                    Style::default().fg(t.muted).add_modifier(Modifier::ITALIC),
                ))];
                for text_line in self.thinking.accumulated.lines() {
                    lines.push(Line::from(Span::styled(
                        format!("  {text_line}"),
                        Style::default().fg(t.muted).add_modifier(Modifier::ITALIC),
                    )));
                }
                lines
            } else {
                // Collapsed: single line with animated dots
                let bright = self.thinking.bright_dot_index();
                let mut spans = vec![Span::styled(
                    "+ Thinking",
                    Style::default().fg(t.thinking_dot),
                )];
                for i in 0..3u8 {
                    let color = if i as usize == bright {
                        t.thinking_dot_bright
                    } else {
                        t.thinking_dot
                    };
                    spans.push(Span::styled(".", Style::default().fg(color)));
                }
                vec![Line::from(spans)]
            }
        } else if self.thinking.last_duration_ms > 0 && !self.thinking.expanded {
            // Completed, collapsed summary — always shown regardless of show_thinking
            let ms = self.thinking.last_duration_ms;
            let duration_str = if ms >= 1000 {
                format!("{:.1}s", ms as f64 / 1000.0)
            } else {
                format!("{ms}ms")
            };
            if self.thinking.has_content() {
                let chars = self.thinking.accumulated.len();
                vec![Line::from(Span::styled(
                    format!("+ Thought for {duration_str} ({chars} chars)"),
                    Style::default().fg(t.muted),
                ))]
            } else {
                // Thinking text was hidden, but still show the duration
                vec![Line::from(Span::styled(
                    format!("+ Thought for {duration_str}"),
                    Style::default().fg(t.muted),
                ))]
            }
        } else if self.thinking.has_content() && self.thinking.expanded {
            // Completed but user expanded
            let mut lines = vec![Line::from(Span::styled(
                "- Thinking (complete):",
                Style::default().fg(t.muted).add_modifier(Modifier::ITALIC),
            ))];
            for text_line in self.thinking.accumulated.lines() {
                lines.push(Line::from(Span::styled(
                    format!("  {text_line}"),
                    Style::default().fg(t.muted).add_modifier(Modifier::ITALIC),
                )));
            }
            lines
        } else {
            Vec::new()
        }
    }
}

/// A session entry for the /resume picker.
#[derive(Debug, Clone)]
pub struct SessionPickerEntry {
    pub id: String,
    pub name: String,
    pub turns: u64,
    pub cost: f64,
    pub last_active: String,
}

/// Interactive session picker state (shown as modal overlay on /resume).
#[derive(Debug, Clone)]
pub struct SessionPicker {
    pub sessions: Vec<SessionPickerEntry>,
    pub selected: usize,
}

/// An MCP server entry shown in the MCP manager overlay.
#[derive(Debug, Clone)]
pub struct McpServerEntry {
    pub name: String,
    /// One of: "ready", "crashed", "starting", "stopped", "disabled".
    pub state: String,
    pub tool_count: usize,
    pub disabled: bool,
    /// Fully-qualified tool names (mcp__server__tool) for View Tools.
    pub tools: Vec<String>,
}

/// Which sub-view is active inside the MCP manager overlay.
#[derive(Debug, Clone)]
pub enum McpManagerView {
    ServerList {
        selected: usize,
    },
    ServerMenu {
        server_idx: usize,
        action_idx: usize,
    },
    /// Scrollable list of tool names for a specific server.
    ToolList {
        server_name: String,
        tools: Vec<String>,
        scroll: usize,
    },
}

/// Interactive MCP server manager state (shown as modal overlay on /mcp).
#[derive(Debug, Clone)]
pub struct McpManager {
    pub servers: Vec<McpServerEntry>,
    pub view: McpManagerView,
}

/// Configuration for the splash screen passed in from main.
#[derive(Debug, Clone, Default)]
pub struct SplashConfig {
    /// Model name to display.
    pub model: String,
    /// Working directory to display.
    pub working_dir: String,
    /// Recent session activity.
    pub activity: Vec<ActivityEntry>,
}

/// Run the TUI event loop.
///
/// - `event_rx`: receives events from the agent loop
/// Returns `true` when a [`KeyEvent`] should be processed.
///
/// On Windows, crossterm emits both `Press` and `Release` events for every
/// keystroke.  We accept `Press` and `Repeat` (for held keys like backspace
/// and arrows) but discard `Release` to avoid double input.
pub fn should_process_key_event(key: &KeyEvent) -> bool {
    key.kind != KeyEventKind::Release
}

/// - `input_tx`: sends user input to the agent loop
/// - `splash`: optional splash-screen configuration
/// - `stdin_event_rx`: receives stdin events from a spawned task (TUI-310)
///
/// This function takes over the terminal and returns when the user quits.
pub async fn run_tui(
    mut event_rx: tokio::sync::mpsc::Receiver<TuiEvent>,
    input_tx: InputSender,
    splash: Option<SplashConfig>,
    btw_tx: Option<tokio::sync::mpsc::Sender<String>>,
    permission_tx: Option<tokio::sync::mpsc::Sender<bool>>,
    mut stdin_event_rx: tokio::sync::mpsc::Receiver<crossterm::event::Event>,
) -> Result<(), io::Error> {
    // Setup terminal - TerminalGuard handles raw mode, alternate screen, and cursor hide.
    // Its Drop will restore the terminal on function exit.
    let _guard = TerminalGuard::enter()?;
    // Mouse capture enabled for scroll support. Most terminals let you hold Shift
    // while dragging to select text even with mouse capture active (works in
    // Windows Terminal, WezTerm, Kitty, iTerm2, GNOME Terminal, etc.).
    // Use /copy or Ctrl+Y to copy the last assistant response to clipboard.
    io::stdout().execute(EnableMouseCapture)?;
    let backend = CrosstermBackend::new(io::stdout());
    let mut terminal = Terminal::new(backend)?;

    let mut app = App::new();
    if let Some(cfg) = splash {
        app.splash_model = cfg.model;
        app.splash_working_dir = cfg.working_dir;
        app.splash_activity = cfg.activity;
    }

    // Animation tick interval
    let mut tick_interval = tokio::time::interval(std::time::Duration::from_millis(80));
    tick_interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    loop {
        // Draw UI - blocking call is OK on Tokio's threaded runtime
        // (other tasks run on other threads while this one is blocked)
        terminal.draw(|frame| { crate::render::draw(frame, &mut app) })?;

        // Drain agent events first (non-blocking)
        while let Ok(tui_event) = event_rx.try_recv() {
            handle_tui_event(&mut app, tui_event, &input_tx);
            if app.should_quit {
                break;
            }
        }

        if app.should_quit {
            break;
        }

        // Use tokio::select! to wait for stdin OR tick
        // Biased: agent events are processed first (see biased! below)
        tokio::select! {
            biased;

            // Stdin events from the spawned task
            Some(stdin_event) = stdin_event_rx.recv() => {
                match stdin_event {
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
                        handle_key_event(&mut app, key, &input_tx, &btw_tx, &permission_tx);
                    }
                    Event::Resize(cols, rows) => {
                        crate::layout::handle_resize(cols, rows);
                    }
                    _ => {} // FocusGained/FocusLost/Paste
                }
            }

            // Animation tick
            _ = tick_interval.tick() => {
                // Tick animations - rate-limited to reduce CPU
                app.input.ultrathink.tick();
                app.thinking.tick_thinking();
            }
        }
    }

    // Restore terminal - DisableMouseCapture only; TerminalGuard's Drop handles
    // cursor show, leave alternate screen, and disable raw mode.
    io::stdout().execute(DisableMouseCapture)?;

    Ok(())
}

/// Handle a TuiEvent dispatched from the agent loop.
fn handle_tui_event(
    app: &mut App,
    tui_event: TuiEvent,
    input_tx: &tokio::sync::mpsc::Sender<String>,
) {
    match tui_event {
        TuiEvent::TextDelta(text) => app.on_text_delta(&text),
        TuiEvent::ThinkingDelta(text) => app.on_thinking_delta(&text),
        TuiEvent::ToolStart { name, id } => app.on_tool_start(&name, &id),
        TuiEvent::ToolComplete {
            name,
            id,
            success,
            output,
        } => {
            app.on_tool_complete(&name, &id, success, &output);
        }
        TuiEvent::TurnComplete {
            input_tokens,
            output_tokens,
        } => {
            app.on_turn_complete();
            app.status.cost +=
                (input_tokens as f64 * 3.0 + output_tokens as f64 * 15.0) / 1_000_000.0;
            let queued: Vec<String> = app.pending_input.drain(..).collect();
            for text in queued {
                let _ = input_tx.try_send(text);
            }
        }
        TuiEvent::Error(msg) => app.on_error(&msg),
        TuiEvent::GenerationStarted => app.on_generation_started(),
        TuiEvent::SlashCommandComplete => app.on_slash_command_complete(),
        TuiEvent::ThinkingToggle(enabled) => {
            app.show_thinking = enabled;
        }
        TuiEvent::ModelChanged(model) => {
            app.status.model = model;
        }
        TuiEvent::BtwResponse(response) => {
            app.btw_overlay = Some(response);
        }
        TuiEvent::PermissionPrompt {
            tool,
            description: _,
        } => {
            app.permission_prompt = Some(tool);
        }
        TuiEvent::SessionRenamed(name) => {
            app.session_name = Some(name);
        }
        TuiEvent::PermissionModeChanged(mode) => {
            app.status.permission_mode = mode;
        }
        TuiEvent::ShowSessionPicker(sessions) => {
            app.session_picker = Some(SessionPicker {
                sessions,
                selected: 0,
            });
        }
        TuiEvent::SetAccentColor(color) => {
            app.theme.accent = color;
            app.theme.header = color;
            app.theme.border_active = color;
            app.theme.thinking_dot = color;
        }
        TuiEvent::SetTheme(name) => {
            if let Some(t) = crate::theme::theme_by_name(&name) {
                app.theme = t;
            }
        }
        TuiEvent::ShowMcpManager(servers) => {
            app.mcp_manager = Some(McpManager {
                servers,
                view: McpManagerView::ServerList { selected: 0 },
            });
        }
        TuiEvent::UpdateMcpManager(servers) => {
            if let Some(ref mut mgr) = app.mcp_manager {
                mgr.servers = servers;
            }
        }
        TuiEvent::SetVimMode(enabled) => {
            if enabled {
                app.vim_state = Some(VimState::new());
            } else {
                app.vim_state = None;
            }
        }
        TuiEvent::VimToggle => {
            if app.vim_state.is_some() {
                app.vim_state = None;
            } else {
                app.vim_state = Some(VimState::new());
            }
        }
        TuiEvent::VoiceText(text) => {
            app.input.inject_text(&text);
        }
        TuiEvent::SetAgentInfo { name, color } => {
            app.status.agent_name = Some(name);
            app.status.agent_color = color;
        }
        TuiEvent::Resize { cols, rows } => {
            crate::layout::handle_resize(cols, rows);
        }
        TuiEvent::UserInput(_) => {
            // TUI-106: handled by run_event_loop; old run_tui path is a no-op.
        }
        TuiEvent::SlashCancel => {
            // TUI-106: handled by run_event_loop; old run_tui path is a no-op.
        }
        TuiEvent::SlashAgent(_) => {
            // TUI-106: handled by run_event_loop; old run_tui path is a no-op.
        }
        TuiEvent::Done => {
            app.should_quit = true;
        }
    }
}

/// Handle a key event from stdin.
fn handle_key_event(
    app: &mut App,
    key: KeyEvent,
    input_tx: &tokio::sync::mpsc::Sender<String>,
    btw_tx: &Option<tokio::sync::mpsc::Sender<String>>,
    permission_tx: &Option<tokio::sync::mpsc::Sender<bool>>,
) {
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
                    let _ = input_tx.try_send(format!("__resume_session__ {}", s.id));
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
                            let action_count = mcp_action_count(mgr.servers.get(*server_idx));
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
                                            let _ = input_tx.try_send(cmd);
                                        }
                                    }
                                }
                            }
                        }
                        McpManagerView::ToolList { .. } => {}
                    }
                }
                return;
            }
            KeyCode::Esc => {
                if let Some(ref mut mgr) = app.mcp_manager {
                    match &mgr.view {
                        McpManagerView::ToolList { server_name, .. } => {
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
            _ => return,
        }
    }

    // Handle permission prompt — y/n/Enter/Esc
    if app.permission_prompt.is_some() {
        match key.code {
            KeyCode::Char('y') | KeyCode::Char('Y') | KeyCode::Enter => {
                let tool = app.permission_prompt.take().unwrap_or_default();
                if let Some(tx) = permission_tx {
                    let _ = tx.try_send(true);
                }
                app.output.append_line(&format!("[{tool}: approved]"));
                return;
            }
            KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => {
                let tool = app.permission_prompt.take().unwrap_or_default();
                if let Some(tx) = permission_tx {
                    let _ = tx.try_send(false);
                }
                app.output.append_line(&format!("[{tool}: denied]"));
                return;
            }
            _ => return,
        }
    }

    // Dismiss /btw overlay on any of Esc/Enter/Space
    if app.btw_overlay.is_some() {
        match key.code {
            KeyCode::Esc | KeyCode::Enter | KeyCode::Char(' ') => {
                app.btw_overlay = None;
                return;
            }
            _ => return,
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
                        let _ = input_tx.try_send(text);
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

    match key {
        // Ctrl+D = quit
        KeyEvent {
            code: KeyCode::Char('d'),
            modifiers: KeyModifiers::CONTROL,
            ..
        } => {
            app.should_quit = true;
        }
        // Ctrl+C = interrupt generation or quit
        KeyEvent {
            code: KeyCode::Char('c'),
            modifiers: KeyModifiers::CONTROL,
            ..
        } => {
            if app.is_generating {
                app.is_generating = false;
                app.output.append_line("[interrupted]");
                let _ = input_tx.try_send("__cancel__".to_string());
            } else {
                app.should_quit = true;
            }
        }
        // Ctrl+T = toggle thinking expand
        KeyEvent {
            code: KeyCode::Char('t'),
            modifiers: KeyModifiers::CONTROL,
            ..
        } => {
            app.thinking.toggle_expand();
        }
        // Ctrl+V = voice hotkey
        KeyEvent {
            code: KeyCode::Char('v'),
            modifiers: KeyModifiers::CONTROL,
            ..
        } => {
            crate::voice::pipeline::fire_trigger_for_hotkey();
        }
        // Ctrl+\ = toggle split pane
        KeyEvent {
            code: KeyCode::Char('\\'),
            modifiers: KeyModifiers::CONTROL,
            ..
        } => {
            app.panes.toggle_split();
        }
        // Ctrl+W = switch pane focus
        KeyEvent {
            code: KeyCode::Char('w'),
            modifiers: KeyModifiers::CONTROL,
            ..
        } => {
            app.panes.switch_focus();
        }
        // Page Up = scroll up
        KeyEvent {
            code: KeyCode::PageUp,
            ..
        } => {
            app.output.scroll_up(10);
        }
        // Page Down = scroll down
        KeyEvent {
            code: KeyCode::PageDown,
            ..
        } => {
            app.output.scroll_down(10);
        }
        // Home = scroll to top
        KeyEvent {
            code: KeyCode::Home,
            modifiers: KeyModifiers::CONTROL,
            ..
        } => {
            app.output.scroll_offset = u16::MAX;
            app.output.scroll_locked = true;
        }
        // End = scroll to bottom
        KeyEvent {
            code: KeyCode::End,
            modifiers: KeyModifiers::CONTROL,
            ..
        } => {
            app.output.scroll_to_bottom();
        }
        // Esc = dismiss suggestions, or double-Esc to cancel generation
        KeyEvent {
            code: KeyCode::Esc, ..
        } => {
            if app.is_generating {
                let now = std::time::Instant::now();
                if let Some(last) = app.last_esc
                    && now.duration_since(last).as_millis() < 500
                {
                    app.is_generating = false;
                    app.active_tool = None;
                    app.output.append_line("[interrupted]");
                    app.last_esc = None;
                } else {
                    app.last_esc = Some(now);
                }
            } else {
                app.input.dismiss_suggestions();
            }
        }
        // Shift+Tab = cycle permission mode
        KeyEvent {
            code: KeyCode::BackTab,
            ..
        } => {
            let current = &app.status.permission_mode;
            let modes = [
                "default",
                "acceptEdits",
                "plan",
                "auto",
                "dontAsk",
                "bypassPermissions",
            ];
            let idx = modes.iter().position(|m| m == current).unwrap_or(0);
            let next = modes[(idx + 1) % modes.len()];
            app.status.permission_mode = next.to_string();
            let _ = input_tx.try_send(format!("/permissions {next}"));
        }
        // Tab = accept selected suggestion
        KeyEvent {
            code: KeyCode::Tab,
            modifiers: KeyModifiers::NONE,
            ..
        } => {
            app.input.accept_suggestion();
        }
        // Enter = submit input (queue if generating)
        KeyEvent {
            code: KeyCode::Enter,
            modifiers: KeyModifiers::NONE,
            ..
        } => {
            if app.input.suggestions.active {
                let is_exact_match = app
                    .input
                    .suggestions
                    .suggestions
                    .iter()
                    .any(|cmd| cmd.name == app.input.text());
                if is_exact_match {
                    app.input.dismiss_suggestions();
                } else {
                    app.input.accept_suggestion();
                    return;
                }
            }
            let text = app.submit_input();
            if !text.is_empty() {
                if text.starts_with("/btw ") {
                    if let Some(btw) = btw_tx {
                        let question = text
                            .strip_prefix("/btw ")
                            .unwrap_or("")
                            .trim()
                            .to_string();
                        if !question.is_empty() {
                            let _ = btw.try_send(question);
                        }
                    } else {
                        let _ = input_tx.try_send(text);
                    }
                } else if app.is_generating {
                    app.pending_input.push(text);
                    app.output
                        .append_line("[queued — will send after current turn]");
                } else {
                    let _ = input_tx.try_send(text);
                }
            }
        }
        // Backspace
        KeyEvent {
            code: KeyCode::Backspace,
            ..
        } => {
            app.input.backspace();
        }
        // Up arrow = navigate suggestions or history
        KeyEvent {
            code: KeyCode::Up, ..
        } => {
            if app.input.suggestions.active {
                app.input.suggestions.select_prev();
            } else {
                app.input.history_up();
            }
        }
        // Down arrow = navigate suggestions or history
        KeyEvent {
            code: KeyCode::Down,
            ..
        } => {
            if app.input.suggestions.active {
                app.input.suggestions.select_next();
            } else {
                app.input.history_down();
            }
        }
        // Left arrow
        KeyEvent {
            code: KeyCode::Left,
            ..
        } => app.input.move_left(),
        // Right arrow
        KeyEvent {
            code: KeyCode::Right,
            ..
        } => app.input.move_right(),
        // Regular character input
        KeyEvent {
            code: KeyCode::Char(c),
            modifiers: KeyModifiers::NONE | KeyModifiers::SHIFT,
            ..
        } => {
            app.input.insert(c);
        }
        _ => {}
    }
}

/// Configuration for app::run() — the single event loop entry point (TUI-310).
///
/// Bundles everything needed to set up the TUI event loop. main.rs parses
/// args and constructs this config, then calls app::run().
pub struct AppConfig {
    pub event_rx: tokio::sync::mpsc::Receiver<TuiEvent>,
    pub input_tx: tokio::sync::mpsc::Sender<String>,
    pub splash: Option<SplashConfig>,
    pub btw_tx: Option<tokio::sync::mpsc::Sender<String>>,
    pub permission_tx: Option<tokio::sync::mpsc::Sender<bool>>,
    pub vim_mode: bool,
}

impl AppConfig {
    /// Create an AppConfig from the given channels and options.
    pub fn new(
        event_rx: tokio::sync::mpsc::Receiver<TuiEvent>,
        input_tx: tokio::sync::mpsc::Sender<String>,
        splash: Option<SplashConfig>,
        btw_tx: Option<tokio::sync::mpsc::Sender<String>>,
        permission_tx: Option<tokio::sync::mpsc::Sender<bool>>,
        vim_mode: bool,
    ) -> Self {
        Self {
            event_rx,
            input_tx,
            splash,
            btw_tx,
            permission_tx,
            vim_mode,
        }
    }
}

/// The single TUI event loop entry point (TUI-310).
///
/// Sets up:
/// - SIGWINCH handler for resize events
/// - Blocking task for stdin event reading (crossterm event::poll)
/// - Main loop via run_tui()
///
/// Returns when the user quits.
pub async fn run(config: AppConfig) -> anyhow::Result<()> {
    // Create channel for stdin events (sender goes to blocking task, receiver goes to run_tui)
    let (stdin_event_tx, stdin_event_rx) = tokio::sync::mpsc::channel::<crossterm::event::Event>(32);

    // Spawn blocking task that runs event::poll() and sends events
    // This avoids blocking the async executor on the synchronous crossterm API
    tokio::task::spawn_blocking(move || {
        // Run a local event loop on this blocking thread
        loop {
            // Use a short timeout to allow checking for shutdown
            // The actual rate-limiting is done by the tick_interval in run_tui
            let timeout = std::time::Duration::from_millis(100);
            if crossterm::event::poll(timeout).is_err() {
                // Error reading events — terminal may have been closed
                break;
            }
            // Read the event (poll returned true, so this should not block)
            match crossterm::event::read() {
                Ok(event) => {
                    if stdin_event_tx.blocking_send(event).is_err() {
                        // Receiver dropped — shutdown
                        break;
                    }
                }
                Err(_) => {
                    // Error reading event — terminal may have been closed
                    break;
                }
            }
        }
    });

    // Run the TUI loop
    run_tui(
        config.event_rx,
        config.input_tx,
        config.splash,
        config.btw_tx,
        config.permission_tx,
        stdin_event_rx,
    )
    .await?;

    Ok(())
}

/// Return the action strings available for a given server entry.
///
/// The order is significant — it's the display order in the menu.
pub fn mcp_actions_for(server: &McpServerEntry) -> Vec<&'static str> {
    let mut actions: Vec<&'static str> = Vec::new();
    if server.disabled {
        actions.push("enable");
    } else {
        if matches!(server.state.as_str(), "crashed" | "stopped") {
            actions.push("reconnect");
        }
        if server.state == "ready" {
            actions.push("tools");
        }
        actions.push("disable");
    }
    actions.push("back");
    actions
}

/// Return the number of actions for a server (used for Down key wrap).
pub fn mcp_action_count(server: Option<&McpServerEntry>) -> usize {
    match server {
        Some(s) => mcp_actions_for(s).len(),
        None => 1, // just "back"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn app_text_delta() {
        let mut app = App::new();
        app.on_text_delta("Hello ");
        app.on_text_delta("world");
        assert_eq!(app.output.all_lines(), vec!["Hello world"]);
    }

    #[test]
    fn app_submit_input_does_not_set_generating() {
        let mut app = App::new();
        app.input.insert('t');
        app.input.insert('e');
        app.input.insert('s');
        app.input.insert('t');
        let text = app.submit_input();
        assert_eq!(text, "test");
        // submit_input never sets is_generating — that is done by
        // GenerationStarted event from main.rs before agent.process_message()
        assert!(!app.is_generating);
    }

    #[test]
    fn app_tool_lifecycle() {
        let mut app = App::new();
        // GenerationStarted sets is_generating (not on_tool_start)
        app.on_generation_started();
        assert!(app.is_generating);
        app.on_tool_start("Read", "tool-123");
        assert_eq!(app.active_tool.as_deref(), Some("Read"));
        app.on_tool_complete("Read", "tool-123", true, "file contents here");
        assert!(app.active_tool.is_none());
        // Successful tool calls do NOT append to output (no noise)
        assert!(app.output.all_lines().is_empty());
        // But the tool output state is tracked
        assert_eq!(app.tool_outputs.len(), 1);
        assert_eq!(app.tool_outputs[0].tool_name, "Read");
    }

    #[test]
    fn app_tool_failure_shows_in_output() {
        let mut app = App::new();
        app.on_tool_start("Bash", "tool-456");
        app.on_tool_complete("Bash", "tool-456", false, "command not found");
        // Failed tool calls DO show in output
        assert!(
            app.output
                .all_lines()
                .iter()
                .any(|l| l.contains("Bash") && l.contains("failed"))
        );
    }

    #[test]
    fn thinking_delta_does_not_pollute_output() {
        let mut app = App::new();
        app.show_thinking = true;
        app.on_thinking_delta("I am pondering...");
        // Output buffer should be empty — thinking goes to ThinkingState
        assert!(app.output.all_lines().is_empty());
        assert!(app.thinking.active);
        assert_eq!(app.thinking.accumulated, "I am pondering...");
    }

    #[test]
    fn thinking_tracks_timing_even_when_hidden() {
        let mut app = App::new();
        // show_thinking is false by default
        app.on_thinking_delta("hidden thought");
        assert!(app.thinking.active);
        assert!(app.thinking.start.is_some());
        // Text NOT accumulated when hidden
        assert!(app.thinking.accumulated.is_empty());
    }

    #[test]
    fn thinking_completes_on_text_delta() {
        let mut app = App::new();
        app.show_thinking = true;
        app.on_thinking_delta("deep thought");
        assert!(app.thinking.active);
        app.on_text_delta("answer");
        // Thinking should now be complete; summary is rendered by
        // thinking_lines(), NOT appended to the output buffer.
        assert!(!app.thinking.active);
        let lines = app.output.all_lines();
        assert!(!lines.iter().any(|l| l.contains("Thought for")));
        assert!(lines.iter().any(|l| l.contains("answer")));
    }

    #[test]
    fn thinking_completes_on_turn_complete() {
        let mut app = App::new();
        app.on_thinking_delta("pondering");
        app.on_turn_complete();
        assert!(!app.thinking.active);
        // Summary is rendered separately — not in the output buffer.
        let lines = app.output.all_lines();
        assert!(!lines.iter().any(|l| l.contains("Thought for")));
    }

    #[test]
    fn submit_input_never_sets_is_generating() {
        // No input — slash or normal — should set is_generating in submit_input.
        // The flag is controlled exclusively by GenerationStarted/TurnComplete events.
        let cases = vec![
            "hello world",
            "/model opus",
            "/fast",
            "/gibberish",
            "/",
            "/ help",
            "/usr/bin/foo",
            "/etc/hosts",
        ];
        for input in cases {
            let mut app = App::new();
            for c in input.chars() {
                app.input.insert(c);
            }
            let text = app.submit_input();
            assert_eq!(text, input);
            assert!(
                !app.is_generating,
                "submit_input set is_generating for '{input}'"
            );
        }
    }

    #[test]
    fn generation_started_sets_is_generating() {
        let mut app = App::new();
        assert!(!app.is_generating);
        app.on_generation_started();
        assert!(app.is_generating);
    }

    #[test]
    fn slash_command_complete_resets_is_generating() {
        let mut app = App::new();
        app.on_slash_command_complete();
        assert!(!app.is_generating);
    }

    #[test]
    fn full_agent_turn_lifecycle() {
        // Simulates: user submits -> GenerationStarted -> TextDelta -> TurnComplete
        let mut app = App::new();
        for c in "hello".chars() {
            app.input.insert(c);
        }
        app.submit_input();
        assert!(!app.is_generating); // submit_input does NOT set it

        app.on_generation_started();
        assert!(app.is_generating); // now set by event

        app.on_text_delta("response");
        assert!(app.is_generating); // still generating during response

        app.on_turn_complete();
        assert!(!app.is_generating); // reset after turn completes
    }

    #[test]
    fn slash_command_lifecycle() {
        // Simulates: user submits /model -> SlashCommandComplete
        let mut app = App::new();
        for c in "/model opus".chars() {
            app.input.insert(c);
        }
        app.submit_input();
        assert!(!app.is_generating); // never set for slash commands

        // main.rs sends SlashCommandComplete — this is a no-op since
        // is_generating was never true, but it ensures consistency
        app.on_slash_command_complete();
        assert!(!app.is_generating);
    }

    #[test]
    fn unrecognized_slash_command_fallthrough() {
        // Simulates: user types /gibberish -> not handled -> falls through to agent
        let mut app = App::new();
        for c in "/gibberish".chars() {
            app.input.insert(c);
        }
        app.submit_input();
        assert!(!app.is_generating); // submit_input does NOT set it

        // main.rs sends GenerationStarted before agent.process_message()
        app.on_generation_started();
        assert!(app.is_generating); // correctly set for agent turn

        app.on_turn_complete();
        assert!(!app.is_generating);
    }
}
