use std::io;

use crossterm::ExecutableCommand;
use crossterm::event::{DisableMouseCapture, EnableMouseCapture, KeyEvent, KeyEventKind};
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};

use crate::agent_activity::AgentActivityRow;
use crate::events::AgentActivityUpdate;
use crate::input::InputHandler;
use crate::output::{OutputBuffer, ThinkingState, ToolOutputState};
use crate::splash::ActivityEntry;
use crate::split_pane::SplitPaneManager;
use crate::status::StatusBar;
use crate::terminal::TerminalGuard;
use crate::theme::{Theme, intj_theme};
use crate::vim::VimState;

// Re-export layer-0 event payloads so existing `archon_tui::app::*` imports
// remain stable while `crate::events` stays the canonical source.
pub use crate::events::{
    AgentActivityRole, EvidenceRowPayload, FileEntry, McpServerEntry, MessageSummary,
    SessionPickerEntry, SkillEntry, TuiEvent, VideoIngestProgressEvent, ViewId,
};
pub use crate::evidence_view_state::EvidenceViewState;

// REM-2d: Modal overlay state types relocated to sibling module
// `crate::app_modals` (docs/rem-2-split-plan.md §7, Option 7A). The
// `archon_tui::app::{SessionPicker, McpManager, McpManagerView, SplashConfig}`
// path is preserved via this re-export so downstream callers are untouched.
pub use crate::app_modals::{McpManager, McpManagerView, SessionPicker, SplashConfig};

/// Callback type for sending user input to the agent loop.
pub type InputSender = tokio::sync::mpsc::Sender<String>;

/// Configuration for launching the TUI session.
/// Passed from main.rs to app::run().
pub struct AppConfig {
    pub event_rx: crate::event_channel::TuiEventReceiver,
    pub input_tx: InputSender,
    pub model: String,
    pub splash: Option<SplashConfig>,
    pub btw_tx: Option<tokio::sync::mpsc::Sender<String>>,
    pub permission_tx: Option<tokio::sync::mpsc::Sender<bool>>,
    pub context_window: u64,
    pub context_source: Option<String>,
    pub context_threshold: f32,
    /// Command catalog injected from the bin crate's registry so autocomplete
    /// stays locked to `Registry::primaries_with_descriptions()`.
    pub command_catalog: Vec<crate::commands::CommandInfo>,
}

/// Thin entry point that sets up terminal infrastructure and delegates to
/// [`crate::event_loop::run_inner`]. The public API called from `main.rs`.
pub async fn run(config: AppConfig) -> Result<(), io::Error> {
    // Setup terminal - TerminalGuard handles raw mode, alternate screen, and cursor hide.
    // Its Drop will restore the terminal on function exit.
    let _guard = TerminalGuard::enter()?;
    // Keep normal terminal text selection available by default. Operators who
    // prefer mouse-wheel events inside the TUI can opt into capture with
    // ARCHON_TUI_MOUSE_CAPTURE=1.
    let mouse_capture = crate::terminal::mouse_capture_enabled();
    if mouse_capture {
        io::stdout().execute(EnableMouseCapture)?;
    }
    let backend = CrosstermBackend::new(io::stdout());
    let mut terminal = Terminal::new(backend)?;

    // TASK-TUI-406: spawn BACKGROUND_AGENTS GC janitor at startup (60s
    // interval). Detached — task runs for TUI session lifetime.
    // Accessed via archon_core's re-export (archon-tools is dev-only dep).
    let _gc_handle = archon_core::background_agents::spawn_gc_task();

    let result = crate::event_loop::run_inner(config, &mut terminal).await;

    // Restore terminal - TerminalGuard's Drop handles cursor show, leave
    // alternate screen, bracketed paste, and raw mode.
    if mouse_capture {
        io::stdout().execute(DisableMouseCapture)?;
    }

    result
}

/// Backend-injection seam for integration tests (TUI-327).
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
    pub active_tool: Option<String>,
    pub tool_outputs: Vec<ToolOutputState>,
    pub agent_activity: Vec<AgentActivityRow>,
    pub activity_stream: crate::activity_stream::ActivityStreamState,
    pub show_thinking: bool,
    last_esc: Option<std::time::Instant>,
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
    pub message_selector: Option<crate::screens::message_selector::MessageSelector>,
    /// TASK-TUI-627: active skills-menu modal (shown by /skills).
    pub skills_menu: Option<crate::screens::skills_menu::SkillsMenu>,
    /// TASK-#207 SLASH-FILES: active file-picker modal (shown by /files).
    pub file_picker: Option<crate::screens::file_picker::FilePicker>,
    /// TASK-#208 SLASH-SEARCH: active search-results modal (shown by /search).
    pub search_results: Option<crate::screens::search_results::SearchResults>,
    /// Evidence Engine inspection overlay opened by TuiEvent::OpenView.
    pub evidence_view: Option<EvidenceViewState>,
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
            agent_activity: Vec::new(),
            activity_stream: crate::activity_stream::ActivityStreamState::default(),
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
            file_picker: None,
            search_results: None,
            evidence_view: None,
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

    pub fn input_accepts_paste(&self) -> bool {
        self.permission_prompt.is_none()
            && self.btw_overlay.is_none()
            && self.session_picker.is_none()
            && self.mcp_manager.is_none()
            && self.message_selector.is_none()
            && self.skills_menu.is_none()
            && self.file_picker.is_none()
            && self.search_results.is_none()
            && self.evidence_view.is_none()
            && self.vim_state.is_none()
            && !self.activity_stream.is_foreground()
    }

    pub fn on_text_delta(&mut self, text: &str) {
        // A non-thinking event while thinking is active means thinking ended.
        if self.thinking.active {
            self.finish_thinking();
        }
        self.push_parent_activity_text(text);
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
        self.push_parent_activity_thinking(text);
    }

    pub fn on_tool_start(&mut self, name: &str, id: &str) {
        if self.thinking.active {
            self.finish_thinking();
        }
        // Track active tool for status bar, but don't clutter the output.
        // is_generating is already set by GenerationStarted — not set here.
        self.active_tool = Some(name.to_string());
        self.tool_outputs.push(ToolOutputState::new(name, id));
        self.push_parent_activity_tool_call(name);
        crate::agent_activity::tool_started(&mut self.agent_activity, name, id);
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
        self.push_parent_activity_tool_result(name, output, !success);
        crate::agent_activity::tool_completed(&mut self.agent_activity, name, id, success);
        if !success {
            let output = output.trim_end();
            if output.is_empty() {
                self.output.append_line(&format!("[tool] {name} failed"));
            } else {
                self.output
                    .append_line(&format!("[tool] {name} failed:\n{output}"));
            }
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
        self.push_parent_activity_status("turn complete");
        crate::agent_activity::turn_completed(&mut self.agent_activity);
        // Reset thinking for the next turn.
        self.thinking.reset();
    }

    pub fn on_error(&mut self, message: &str) {
        if self.thinking.active {
            self.finish_thinking();
        }
        self.output.append_line(&format!("[error] {message}"));
        self.is_generating = false;
        self.push_parent_activity_error(message);
        crate::agent_activity::turn_failed(&mut self.agent_activity);
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
        self.push_parent_activity_status("turn started");
        crate::agent_activity::turn_started(&mut self.agent_activity);
    }

    pub fn on_slash_command_complete(&mut self) {
        self.is_generating = false;
    }

    pub fn on_agent_activity(&mut self, update: AgentActivityUpdate) {
        self.record_activity_update(&update);
        crate::context_status::update_actor_context_name(&mut self.status, &update);
        crate::agent_activity::apply_update(&mut self.agent_activity, update);
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
