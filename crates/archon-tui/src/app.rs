use std::io;

use crossterm::ExecutableCommand;
use crossterm::event::{DisableMouseCapture, EnableMouseCapture, KeyEvent, KeyEventKind};
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};

use crate::input::InputHandler;
use crate::output::{OutputBuffer, ThinkingState, ToolOutputState};
use crate::splash::ActivityEntry;
use crate::split_pane::SplitPaneManager;
use crate::status::StatusBar;
use crate::terminal::TerminalGuard;
use crate::theme::{Theme, intj_theme};
use crate::vim::VimState;

// TUI-330: Re-export entry types from their canonical location in
// `crate::events` (layer 0). Moved here from `app.rs` to satisfy the
// module-direction invariant (events.rs may not import from app.rs).
// External consumers that reference `archon_tui::app::McpServerEntry` or
// `archon_tui::app::SessionPickerEntry` remain unbroken via these re-exports.
// TASK-AGS-822: `ViewId` added to the re-export list for the same
// layer-0 reasoning as `McpServerEntry` / `SessionPickerEntry` —
// external consumers (bin-crate command handlers, integration tests)
// reach the enum via `archon_tui::app::ViewId`.
pub use crate::events::{McpServerEntry, MessageSummary, SessionPickerEntry, SkillEntry, ViewId};

// REM-2d: Modal overlay state types relocated to sibling module
// `crate::app_modals` (docs/rem-2-split-plan.md §7, Option 7A). The
// `archon_tui::app::{SessionPicker, McpManager, McpManagerView, SplashConfig}`
// path is preserved via this re-export so downstream callers are untouched.
pub use crate::app_modals::{McpManager, McpManagerView, SessionPicker, SplashConfig};

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
    /// TASK-TUI-620: open the message-selector overlay with a pre-computed
    /// list of MessageSummary entries. The `/rewind` slash command builds
    /// the list from session history and emits this event; the event-loop
    /// arm sets `app.message_selector = Some(...)`. Follow-up ticket wires
    /// input routing + render — for now the overlay is reachable but not
    /// interactive.
    ShowMessageSelector(Vec<MessageSummary>),
    /// TASK-TUI-627: open the skills-menu overlay with a pre-computed
    /// list of SkillEntry. Input/render routing deferred to TUI-627-followup.
    ShowSkillsMenu(Vec<SkillEntry>),
    /// TASK-AGS-822: open an overlay view identified by `ViewId`.
    /// Emitted by the slash-command dispatcher in response to
    /// view-opening commands (`/tasks`, `/settings`, `/context`,
    /// `/memory`, `/model`, `/status`). Clustered with other
    /// overlay-opening variants so future readers locate overlay
    /// events in one place. `ViewId` is defined at layer 0
    /// (`crate::events::ViewId`) and re-exported from this module.
    OpenView(ViewId),
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

/// Configuration for launching the TUI session.
/// Passed from main.rs to app::run().
pub struct AppConfig {
    pub event_rx: tokio::sync::mpsc::Receiver<TuiEvent>,
    pub input_tx: InputSender,
    pub splash: Option<SplashConfig>,
    pub btw_tx: Option<tokio::sync::mpsc::Sender<String>>,
    pub permission_tx: Option<tokio::sync::mpsc::Sender<bool>>,
}

/// Thin entry point that sets up terminal infrastructure and delegates to
/// [`crate::event_loop::run_inner`]. The public API called from `main.rs`.
pub async fn run(config: AppConfig) -> Result<(), io::Error> {
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

    // TASK-TUI-406: spawn BACKGROUND_AGENTS GC janitor at startup (60s
    // interval). Detached — task runs for TUI session lifetime.
    // Accessed via archon_core's re-export (archon-tools is dev-only dep).
    let _gc_handle = archon_core::background_agents::spawn_gc_task();

    let result = crate::event_loop::run_inner(config, &mut terminal).await;

    // Restore terminal - DisableMouseCapture only; TerminalGuard's Drop handles
    // cursor show, leave alternate screen, and disable raw mode.
    io::stdout().execute(DisableMouseCapture)?;

    result
}

/// Backend-injection seam for integration tests (TUI-327). Runs the shared
/// event loop against a caller-owned `Terminal<B>` with **no terminal
/// lifecycle setup** — the caller arranges raw mode / alternate screen /
/// mouse capture as appropriate. Headless backends such as
/// `ratatui::backend::TestBackend` use this entry point; production callers
/// should use [`run`].
pub async fn run_with_backend<B>(
    config: AppConfig,
    terminal: &mut ratatui::Terminal<B>,
) -> Result<(), io::Error>
where
    B: ratatui::backend::Backend,
{
    crate::event_loop::run_inner(config, terminal).await
}

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
    /// TASK-TUI-620: active message-selector modal (shown by /rewind).
    /// TODO(TUI-620-followup): input routing + render wiring deferred.
    pub message_selector: Option<crate::screens::message_selector::MessageSelector>,
    /// TASK-TUI-627: active skills-menu modal (shown by /skills).
    /// TODO(TUI-627-followup): input routing + render wiring deferred.
    pub skills_menu: Option<crate::screens::skills_menu::SkillsMenu>,
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
            message_selector: None,
            skills_menu: None,
            vim_state: None,
            panes: SplitPaneManager::new(),
        }
    }
}

impl App {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn last_esc(&self) -> Option<std::time::Instant> {
        self.last_esc
    }

    pub fn set_last_esc(&mut self, instant: Option<std::time::Instant>) {
        self.last_esc = instant;
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

// TUI-330: `SessionPickerEntry` and `McpServerEntry` moved to `crate::events`
// to honor the layer-0 direction invariant. Both types are re-exported via
// `pub use crate::events::{...}` at the top of this file so the public API
// (`archon_tui::app::SessionPickerEntry` / `archon_tui::app::McpServerEntry`)
// is preserved for `src/session.rs`, `src/command/slash.rs`, and existing
// integration tests.
//
// REM-2d: `SessionPicker`, `McpManagerView`, `McpManager`, `SplashConfig`
// relocated to sibling module `crate::app_modals` to keep `app.rs` under
// the 500-line ceiling. See the `pub use crate::app_modals::{...}` at the
// top of this file.

/// Returns `true` when a [`KeyEvent`] should be processed.
///
/// On Windows, crossterm emits both `Press` and `Release` events for every
/// keystroke. We accept `Press` and `Repeat` (for held keys like backspace
/// and arrows) but discard `Release` to avoid double input.
pub fn should_process_key_event(key: &KeyEvent) -> bool {
    key.kind != KeyEventKind::Release
}

#[cfg(test)]
#[path = "app_tests.rs"]
mod tests;
