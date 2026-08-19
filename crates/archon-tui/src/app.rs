use std::io;

use crossterm::ExecutableCommand;
use crossterm::event::{DisableMouseCapture, EnableMouseCapture, KeyEvent, KeyEventKind};
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;

use crate::agent_activity::AgentActivityRow;
use crate::events::AgentActivityUpdate;
use crate::input::InputHandler;
use crate::output::{OutputBuffer, ThinkingBlock, ThinkingState, ToolOutputState};
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
    pub ask_user_tx: Option<tokio::sync::mpsc::Sender<String>>,
    pub context_window: u64,
    pub context_source: Option<String>,
    pub context_threshold: f32,
    /// Command catalog injected from the bin crate's registry so autocomplete
    /// stays locked to `Registry::primaries_with_descriptions()`.
    pub command_catalog: Vec<crate::commands::CommandInfo>,
    /// Source of rows for the Ctrl+K tasks overlay, and the route back to
    /// cancel one (#189 Phase 9).
    ///
    /// Injected from the bin crate because this crate cannot reach
    /// `archon_tools::task_manager::TASK_MANAGER`. `None` leaves the overlay
    /// unavailable and it says so.
    pub task_store: Option<std::sync::Arc<dyn crate::screens::task_overlay::TaskStore>>,
}

/// Thin entry point that sets up terminal infrastructure and delegates to
/// [`crate::event_loop::run_inner`]. The public API called from `main.rs`.
pub async fn run(config: AppConfig) -> Result<(), io::Error> {
    // Setup terminal - TerminalGuard handles raw mode, alternate screen, and cursor hide.
    // Its Drop will restore the terminal on function exit.
    let _guard = TerminalGuard::enter()?;
    // Keep normal terminal text selection available by default, but auto-capture
    // on WSL because alternate-screen scrollback is unreliable there.
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

/// Headless backend-injection seam for tests that use `TestBackend` and have
/// no crossterm terminal-event source.
pub async fn run_with_backend_without_terminal_events<B>(
    config: AppConfig,
    terminal: &mut ratatui::Terminal<B>,
) -> Result<(), io::Error>
where
    B: ratatui::backend::Backend,
{
    crate::event_loop::run_inner_without_terminal_events(config, terminal).await
}

/// The main TUI application state.
pub struct App {
    pub output: OutputBuffer,
    pub input: InputHandler,
    pub status: StatusBar,
    pub thinking: ThinkingState,
    pub thinking_blocks: Vec<ThinkingBlock>,
    pub thinking_archive: Option<usize>,
    pub theme: Theme,
    /// Name of the applied theme (#192).
    ///
    /// Theme is a colour struct and cannot be reversed to the name that
    /// produced it, so the /theme picker had no way to show which entry is
    /// current — the one question you open it to answer.
    pub theme_name: String,
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
    /// Pending AskUserQuestion prompt, its semantic type, and draft answer.
    pub ask_user_prompt: Option<String>,
    pub ask_user_prompt_kind: Option<archon_core::agent::AskUserPromptKind>,
    pub ask_user_draft: String,
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
    /// `/model` picker overlay (#192). Opened alongside the text summary, so
    /// print mode and scrollback keep the reading they always had.
    pub model_picker: Option<crate::screens::model_picker::ModelPicker>,
    /// `/theme` picker overlay (#192).
    pub theme_screen: Option<crate::screens::theme_screen::ThemeScreen>,
    /// `/hooks` overlay (#192).
    pub hooks_menu: Option<crate::screens::hooks_config_menu::HooksMenu>,
    /// `/permissions` rules overlay (#192). Read-only: nothing at runtime can
    /// change these rules.
    pub permissions_browser: Option<crate::screens::permissions_browser::PermissionsBrowser>,
    /// `/memory files` overlay (#192): the ARCHON.md hierarchy in force.
    pub memory_browser: Option<crate::screens::memory_file_selector::MemoryBrowser>,
    /// `/branch` picker (#192): which message to fork the session from.
    pub branch_picker: Option<crate::screens::session_branching::BranchPicker>,
    /// `/config` settings overlay (#192).
    pub settings_screen: Option<crate::screens::settings_screen::SettingsScreen>,
    /// TASK-#207 SLASH-FILES: active file-picker modal (shown by /files).
    pub file_picker: Option<crate::screens::file_picker::FilePicker>,
    /// TASK-#208 SLASH-SEARCH: active search-results modal (shown by /search).
    pub search_results: Option<crate::screens::search_results::SearchResults>,
    /// #189 Phase 9: tasks overlay (Ctrl+K), listing cancellable background work.
    pub task_overlay: Option<crate::screens::task_overlay::TaskOverlay>,
    /// Source of task rows and the route back to cancel one.
    ///
    /// `None` when nothing injected a store — the overlay then reports that
    /// rather than silently showing an empty list, because "no tasks running"
    /// and "no way to see tasks" are different answers.
    pub task_store: Option<std::sync::Arc<dyn crate::screens::task_overlay::TaskStore>>,
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
            thinking_blocks: Vec::new(),
            thinking_archive: None,
            theme: intj_theme(),
            theme_name: "intj".to_string(),
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
            ask_user_prompt: None,
            ask_user_prompt_kind: None,
            ask_user_draft: String::new(),
            session_name: None,
            session_picker: None,
            mcp_manager: None,
            message_selector: None,
            skills_menu: None,
            model_picker: None,
            theme_screen: None,
            hooks_menu: None,
            permissions_browser: None,
            memory_browser: None,
            branch_picker: None,
            settings_screen: None,
            file_picker: None,
            search_results: None,
            task_overlay: None,
            task_store: None,
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
            && self.ask_user_prompt.is_none()
            && self.btw_overlay.is_none()
            && self.session_picker.is_none()
            && self.mcp_manager.is_none()
            && self.message_selector.is_none()
            && self.thinking_archive.is_none()
            && self.skills_menu.is_none()
            && self.model_picker.is_none()
            && self.theme_screen.is_none()
            && self.hooks_menu.is_none()
            && self.permissions_browser.is_none()
            && self.memory_browser.is_none()
            && self.branch_picker.is_none()
            && self.settings_screen.is_none()
            && self.file_picker.is_none()
            && self.search_results.is_none()
            && self.task_overlay.is_none()
            && self.evidence_view.is_none()
            && self.vim_state.is_none()
            && !self.activity_stream.is_foreground()
    }

    pub fn on_text_delta(&mut self, text: &str) {
        // A non-thinking event while thinking is active means thinking ended.
        if self.thinking.active {
            if self.thinking.transient {
                self.discard_thinking_preview();
            } else {
                self.finish_thinking();
            }
        }
        self.push_parent_activity_text(text);
        self.output.append(text);
    }

    pub fn on_thinking_delta(&mut self, text: &str) {
        if !self.thinking.active {
            self.collapse_all_thinking_blocks();
        }
        self.thinking.on_thinking_delta(text);
        self.push_parent_activity_thinking(text);
    }

    pub fn on_transient_thinking_delta(&mut self, text: &str) {
        if !self.thinking.active {
            self.collapse_all_thinking_blocks();
        }
        self.thinking.on_transient_thinking_delta(text);
    }

    pub fn commit_thinking_preview(&mut self) {
        if self.thinking.active && self.thinking.transient {
            self.thinking.commit_preview();
            self.finish_thinking();
        }
    }

    pub fn discard_thinking_preview(&mut self) {
        self.thinking.discard_preview();
    }

    pub fn on_tool_start(&mut self, name: &str, id: &str) {
        if self.thinking.active && !self.thinking.transient {
            self.finish_thinking();
        }
        // Track active tool for status bar, but don't clutter the output.
        self.active_tool = Some(name.to_string());
        self.tool_outputs.push(ToolOutputState::new(name, id));
        self.push_parent_activity_tool_call(name);
        crate::agent_activity::tool_started(&mut self.agent_activity, name, id);
    }

    pub fn on_tool_complete(&mut self, name: &str, id: &str, success: bool, output: &str) {
        let tool_index = self.tool_outputs.iter().position(|tool| tool.tool_id == id);
        if let Some(index) = tool_index {
            let completing_was_running =
                self.tool_outputs[index].status == crate::output::ToolDisplayStatus::Running;
            if completing_was_running {
                self.active_tool = self.latest_running_tool_except(index);
            }
        }
        let mut completed_output = output.to_string();
        if let Some(index) = tool_index {
            let (duration_ms, summary) = {
                let tool_state = &mut self.tool_outputs[index];
                completed_output = tool_state.complete(output, !success).to_string();
                (tool_state.duration_ms(), tool_state.summary.clone())
            };
            if success {
                let marker_line = self.output.line_count();
                let summary = summary
                    .map(|summary| format!("({summary})"))
                    .unwrap_or_default();
                self.output.append_line(&format!(
                    "● {name}{summary} ✓ {} ({} lines)",
                    format_duration(duration_ms),
                    completed_output.lines().count()
                ));
                self.tool_outputs[index].marker_line = Some(marker_line);
            }
        }
        self.push_parent_activity_tool_result(name, &completed_output, !success);
        crate::agent_activity::tool_completed(&mut self.agent_activity, name, id, success);
        if !success {
            let output = completed_output.trim_end();
            if output.is_empty() {
                self.output.append_line(&format!("[tool] {name} failed"));
            } else {
                self.output
                    .append_line(&format!("[tool] {name} failed:\n{output}"));
            }
        }
    }

    pub fn on_turn_complete(&mut self) {
        if self.thinking.active {
            if self.thinking.transient {
                self.discard_thinking_preview();
            } else {
                self.finish_thinking();
            }
        }
        self.is_generating = false;
        self.output.append_line("");
        self.push_parent_activity_status("turn complete");
        crate::agent_activity::turn_completed(&mut self.agent_activity);
    }

    pub fn on_error(&mut self, message: &str) {
        if self.thinking.active {
            if self.thinking.transient {
                self.discard_thinking_preview();
            } else {
                self.finish_thinking();
            }
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

    /// Finalize the current thinking block exactly once.
    fn finish_thinking(&mut self) {
        if !self.thinking.active {
            return;
        }
        self.thinking.on_thinking_complete();
        let duration_ms = self.thinking.last_duration_ms;
        let text = std::mem::take(&mut self.thinking.accumulated);
        let marker_line = self.output.line_count();
        self.output
            .append_line(&format!("✻ Thought for {}", format_duration(duration_ms)));
        self.thinking_blocks.push(ThinkingBlock {
            text,
            duration_ms,
            marker_line,
            expanded: false,
        });
        self.thinking.expanded = false;
        self.thinking.dot_offset = 0;
    }

    pub fn toggle_thinking(&mut self) {
        if self.thinking.active {
            self.thinking.toggle_expand();
        } else {
            self.toggle_latest_thinking_block();
        }
    }

    // -- rendering helpers --------------------------------------------------

    /// Build the `Line`s for the active thinking indicator (inserted into the
    /// output area at the bottom, before the cursor).
    pub fn thinking_lines(&self, width: u16) -> Vec<ratatui::text::Line<'static>> {
        crate::thinking_view::thinking_lines(self, width)
    }
}

pub(crate) fn format_duration(duration_ms: u64) -> String {
    if duration_ms >= 1000 {
        format!("{:.1}s", duration_ms as f64 / 1000.0)
    } else {
        format!("{duration_ms}ms")
    }
}

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
