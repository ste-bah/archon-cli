use std::io;

use crossterm::ExecutableCommand;
use crossterm::event::{
    self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEvent, KeyModifiers,
    MouseEventKind,
};
use crossterm::terminal::{self, EnterAlternateScreen, LeaveAlternateScreen};
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
use crate::output::{OutputBuffer, ThinkingState};
use crate::splash::{self, ActivityEntry};
use crate::status::StatusBar;
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
        success: bool,
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

    pub fn on_tool_start(&mut self, name: &str, _id: &str) {
        if self.thinking.active {
            self.finish_thinking();
        }
        // Track active tool for status bar, but don't clutter the output.
        // is_generating is already set by GenerationStarted — not set here.
        self.active_tool = Some(name.to_string());
    }

    pub fn on_tool_complete(&mut self, name: &str, success: bool) {
        // Only clear active_tool if it matches the completing tool (guards against overlapping calls)
        if self.active_tool.as_deref() == Some(name) {
            self.active_tool = None;
        }
        if !success {
            // Only show tool failures — they're actionable information
            self.output.append_line(&format!("[tool] {name} failed"));
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
    fn thinking_lines(&self) -> Vec<Line<'_>> {
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
/// - `input_tx`: sends user input to the agent loop
/// - `splash`: optional splash-screen configuration
///
/// This function takes over the terminal and returns when the user quits.
pub async fn run_tui(
    mut event_rx: tokio::sync::mpsc::Receiver<TuiEvent>,
    input_tx: InputSender,
    splash: Option<SplashConfig>,
    btw_tx: Option<tokio::sync::mpsc::Sender<String>>,
    permission_tx: Option<tokio::sync::mpsc::Sender<bool>>,
) -> Result<(), io::Error> {
    // Setup terminal
    terminal::enable_raw_mode()?;
    io::stdout().execute(EnterAlternateScreen)?;
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

    loop {
        // Draw UI
        terminal.draw(|frame| {
            let chunks = Layout::default()
                .direction(Direction::Vertical)
                .constraints([
                    Constraint::Min(3),       // output area
                    Constraint::Length(3),     // input area
                    Constraint::Length(1),     // permission indicator
                    Constraint::Length(1),     // status bar
                ])
                .split(frame.area());

            let t = &app.theme;

            // Output area — splash or buffer lines + thinking indicator
            let output_lines: Vec<Line<'_>> = if app.show_splash {
                splash::render_splash(
                    &app.splash_model,
                    &app.splash_working_dir,
                    &app.splash_activity,
                )
            } else {
                let mut lines: Vec<Line<'_>> = app
                    .output
                    .all_lines()
                    .iter()
                    .map(|line| render_markdown_line(line, &app.theme))
                    .collect();
                lines.extend(app.thinking_lines());
                lines
            };

            let visible_height = chunks[0].height;
            let output_width = chunks[0].width.saturating_sub(1); // -1 for scrollbar
            // Count wrapped rows — logical lines wrap to multiple physical rows
            let raw_strings: Vec<String> = output_lines.iter().map(|l| {
                l.spans.iter().map(|s| s.content.as_ref()).collect::<String>()
            }).collect();
            let raw_refs: Vec<&str> = raw_strings.iter().map(|s| s.as_str()).collect();
            let total_wrapped = OutputBuffer::count_wrapped_rows(&raw_refs, output_width);

            let scroll_y = if app.show_splash {
                0
            } else {
                app.output.effective_scroll(total_wrapped, visible_height)
            };

            // Border color signals when the user is scrolled away from bottom
            let border_style = if app.output.scroll_locked {
                Style::default().fg(t.warning)
            } else {
                Style::default().fg(t.border)
            };

            let output_widget = Paragraph::new(output_lines)
                .block(Block::default().borders(Borders::NONE).style(border_style))
                .wrap(Wrap { trim: false })
                .scroll((scroll_y, 0));
            frame.render_widget(output_widget, chunks[0]);

            // Scrollbar — only shown when content exceeds visible area
            if total_wrapped > visible_height && !app.show_splash {
                let mut scrollbar_state = ScrollbarState::new(total_wrapped.saturating_sub(visible_height) as usize)
                    .position(scroll_y as usize);
                let scrollbar = Scrollbar::new(ScrollbarOrientation::VerticalRight)
                    .style(Style::default().fg(t.muted));
                frame.render_stateful_widget(scrollbar, chunks[0], &mut scrollbar_state);
            }

            // Input area
            let input_border_style = Style::default().fg(if app.is_generating {
                t.border
            } else {
                t.border_active
            });
            let input_widget = if app.input.ultrathink.active {
                // Build per-character rainbow spans for ultrathink keywords
                let text = app.input.text();
                let prefix_span = Span::raw("> ");
                let mut spans = vec![prefix_span];
                for (byte_idx, ch) in text.char_indices() {
                    if let Some(color) = app.input.ultrathink.color_at(byte_idx) {
                        spans.push(Span::styled(
                            String::from(ch),
                            Style::default().fg(color).add_modifier(Modifier::BOLD),
                        ));
                    } else {
                        spans.push(Span::raw(String::from(ch)));
                    }
                }
                Paragraph::new(Line::from(spans))
                    .block(Block::default().borders(Borders::TOP).border_style(input_border_style))
            } else if let Some(ref tool) = app.permission_prompt {
                Paragraph::new(format!("Allow {tool}? [y/n]"))
                    .block(Block::default().borders(Borders::TOP).border_style(
                        Style::default().fg(Color::Yellow)
                    ))
                    .style(Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD))
            } else if let Some(ref vim) = app.vim_state {
                let mode_indicator = vim.mode_display();
                let vim_text = vim.text();
                // Show current line only (last line, where cursor is)
                let display_line = vim_text.lines().last().unwrap_or("").to_string();
                Paragraph::new(format!("{mode_indicator} {display_line}"))
                    .block(Block::default().borders(Borders::TOP).border_style(input_border_style))
                    .style(Style::default().fg(t.accent))
            } else {
                let prefix = if app.is_generating {
                    match &app.active_tool {
                        Some(tool) => format!("[{tool}] > "),
                        None => "[...] > ".to_string(),
                    }
                } else {
                    "> ".to_string()
                };
                Paragraph::new(format!("{prefix}{}", app.input.text()))
                    .block(Block::default().borders(Borders::TOP).border_style(input_border_style))
                    .style(Style::default().fg(t.fg))
            };
            frame.render_widget(input_widget, chunks[1]);

            // Session name badge — right-aligned on the input line
            if let Some(ref name) = app.session_name {
                let badge = format!(" {name} ");
                let badge_width = badge.len() as u16;
                let badge_x = chunks[1].right().saturating_sub(badge_width + 1);
                let badge_area = ratatui::layout::Rect::new(
                    badge_x,
                    chunks[1].y,
                    badge_width,
                    1,
                );
                let badge_widget = Paragraph::new(badge)
                    .style(Style::default().fg(Color::Black).bg(Color::Cyan));
                frame.render_widget(badge_widget, badge_area);
            }

            // Suggestion popup (rendered above the input line)
            if app.input.suggestions.active && !app.is_generating {
                let suggestions = &app.input.suggestions.suggestions;
                let visible_count = suggestions.len().min(8);
                if visible_count > 0 {
                    let selected = app.input.suggestions.selected_index;
                    let items: Vec<ListItem<'_>> = suggestions
                        .iter()
                        .take(8)
                        .enumerate()
                        .map(|(i, cmd)| {
                            let style = if i == selected {
                                Style::default().fg(t.accent).add_modifier(Modifier::BOLD)
                            } else {
                                Style::default().fg(t.fg)
                            };
                            let desc_style = if i == selected {
                                Style::default().fg(t.fg)
                            } else {
                                Style::default().fg(t.muted)
                            };
                            let line = Line::from(vec![
                                Span::styled(
                                    format!("{:<16}", cmd.name),
                                    style,
                                ),
                                Span::styled(cmd.description, desc_style),
                            ]);
                            ListItem::new(line)
                        })
                        .collect();

                    // +2 for top/bottom border
                    let popup_height = (visible_count as u16) + 2;
                    let popup_y = chunks[1].y.saturating_sub(popup_height);
                    let popup_width = chunks[1].width.min(60);
                    let popup_area = Rect::new(
                        chunks[1].x,
                        popup_y,
                        popup_width,
                        popup_height,
                    );

                    let popup = List::new(items)
                        .block(
                            Block::default()
                                .borders(Borders::ALL)
                                .title(" Commands ")
                                .border_style(Style::default().fg(t.border_active))
                                .style(Style::default().fg(t.fg)),
                        );
                    frame.render_widget(popup, popup_area);
                }
            }

            // Status bar (with optional ULTRATHINK rainbow indicator)
            let status_bg = t.border;
            let status_text = app.status.format();
            let status_line = if app.input.ultrathink.active {
                let mut spans: Vec<Span<'_>> = Vec::new();
                // Add rainbow ULTRATHINK label
                for (ch, color) in ultrathink::ultrathink_status_spans(
                    app.input.ultrathink.shimmer_offset,
                ) {
                    spans.push(Span::styled(
                        String::from(ch),
                        Style::default()
                            .fg(color)
                            .bg(status_bg)
                            .add_modifier(Modifier::BOLD),
                    ));
                }
                spans.push(Span::styled(
                    " | ",
                    Style::default()
                        .fg(t.fg)
                        .bg(status_bg)
                        .add_modifier(Modifier::BOLD),
                ));
                spans.push(Span::styled(
                    status_text,
                    Style::default()
                        .fg(t.fg)
                        .bg(status_bg)
                        .add_modifier(Modifier::BOLD),
                ));
                Line::from(spans)
            } else {
                Line::from(Span::styled(
                    status_text,
                    Style::default()
                        .fg(t.fg)
                        .bg(status_bg)
                        .add_modifier(Modifier::BOLD),
                ))
            };
            let status_widget = Paragraph::new(status_line);
            // Permission mode indicator
            let perm_mode = &app.status.permission_mode;
            let perm_display = match perm_mode.as_str() {
                "bypassPermissions" | "yolo" => "bypass permissions on",
                "dontAsk" => "don't ask mode",
                "acceptEdits" => "accept edits mode",
                "auto" => "auto permissions",
                "plan" => "plan mode (read-only)",
                _ => "default permissions",
            };
            let perm_line = Line::from(vec![
                Span::styled(" >> ", Style::default().fg(Color::Yellow)),
                Span::styled(perm_display, Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)),
                Span::styled(" (shift+tab to cycle)", Style::default().fg(t.muted)),
            ]);
            frame.render_widget(Paragraph::new(perm_line), chunks[2]);

            frame.render_widget(status_widget, chunks[3]);

            // /btw overlay — centered modal on top of everything
            if let Some(ref btw_text) = app.btw_overlay {
                let area = frame.area();
                let overlay_width = (area.width * 3 / 4).max(40).min(area.width - 4);
                let lines: Vec<&str> = btw_text.lines().collect();
                let overlay_height = (lines.len() as u16 + 4).min(area.height - 4).max(5);
                let x = (area.width.saturating_sub(overlay_width)) / 2;
                let y = (area.height.saturating_sub(overlay_height)) / 2;
                let overlay_area = Rect::new(x, y, overlay_width, overlay_height);

                // Clear background
                let clear = ratatui::widgets::Clear;
                frame.render_widget(clear, overlay_area);

                let text = format!("{btw_text}\n\n[Esc/Enter to dismiss]");
                let overlay = Paragraph::new(text)
                    .wrap(Wrap { trim: false })
                    .block(
                        Block::default()
                            .borders(Borders::ALL)
                            .title(" /btw ")
                            .border_style(Style::default().fg(t.accent))
                    )
                    .style(Style::default().fg(t.fg));
                frame.render_widget(overlay, overlay_area);
            }

            // Session picker overlay — interactive selection
            if let Some(ref picker) = app.session_picker {
                let area = frame.area();
                let overlay_width = (area.width * 9 / 10).max(70).min(area.width - 2);
                let overlay_height = (picker.sessions.len() as u16 + 3).min(area.height - 4).max(8);
                let x = (area.width.saturating_sub(overlay_width)) / 2;
                let y = (area.height.saturating_sub(overlay_height)) / 2;
                let overlay_area = ratatui::layout::Rect::new(x, y, overlay_width, overlay_height);

                frame.render_widget(ratatui::widgets::Clear, overlay_area);

                let items: Vec<ListItem<'_>> = picker.sessions.iter().enumerate()
                    .map(|(i, s)| {
                        let style = if i == picker.selected {
                            Style::default().fg(Color::Black).bg(Color::Cyan).add_modifier(Modifier::BOLD)
                        } else {
                            Style::default().fg(t.fg)
                        };
                        let id_short = &s.id[..8.min(s.id.len())];
                        let name = if s.name.is_empty() { "-" } else { &s.name };
                        let line = format!(" {id_short} | {name:15} | {:5} turns | ${:5.2} | {}",
                            s.turns, s.cost, s.last_active);
                        ListItem::new(line).style(style)
                    })
                    .collect();

                let list = List::new(items)
                    .block(
                        Block::default()
                            .borders(Borders::ALL)
                            .title(" /resume — select session (Up/Down navigate, Enter select, Esc cancel) ")
                            .border_style(Style::default().fg(t.accent))
                    );
                frame.render_widget(list, overlay_area);
            }

            // MCP manager overlay
            if let Some(ref mcp_mgr) = app.mcp_manager {
                let area = frame.area();
                let overlay_width = (area.width * 3 / 4).max(60).min(area.width - 2);
                let x = (area.width.saturating_sub(overlay_width)) / 2;

                match &mcp_mgr.view {
                    McpManagerView::ServerList { selected } => {
                        let overlay_height = (mcp_mgr.servers.len() as u16 + 3)
                            .max(5)
                            .min(area.height.saturating_sub(4));
                        let y = (area.height.saturating_sub(overlay_height)) / 2;
                        let overlay_area = ratatui::layout::Rect::new(x, y, overlay_width, overlay_height);
                        frame.render_widget(ratatui::widgets::Clear, overlay_area);

                        let items: Vec<ListItem<'_>> = mcp_mgr.servers.iter().enumerate()
                            .map(|(i, s)| {
                                let icon = match s.state.as_str() {
                                    "ready" => "✓",
                                    "crashed" => "✗",
                                    "starting" | "restarting" => "⋯",
                                    "disabled" => "⊘",
                                    _ => "○",
                                };
                                let icon_style = match s.state.as_str() {
                                    "ready" => Style::default().fg(t.accent),
                                    "crashed" => Style::default().fg(Color::Red),
                                    "starting" | "restarting" => Style::default().fg(Color::Yellow),
                                    _ => Style::default().fg(t.muted),
                                };
                                let row_style = if i == *selected {
                                    Style::default().fg(Color::Black).bg(t.accent).add_modifier(Modifier::BOLD)
                                } else {
                                    Style::default().fg(t.fg)
                                };
                                let tool_str = if s.tool_count > 0 {
                                    format!(" ({} tools)", s.tool_count)
                                } else {
                                    String::new()
                                };
                                let line = Line::from(vec![
                                    Span::styled(format!(" {} ", icon), icon_style),
                                    Span::styled(format!("{}{}", s.name, tool_str), row_style),
                                    Span::styled(format!("  [{}]", s.state), Style::default().fg(t.muted)),
                                ]);
                                ListItem::new(line)
                            })
                            .collect();

                        let list = List::new(items)
                            .block(
                                Block::default()
                                    .borders(Borders::ALL)
                                    .title(" MCP Servers (↑↓ navigate, Enter select, Esc close) ")
                                    .border_style(Style::default().fg(t.accent))
                            );
                        frame.render_widget(list, overlay_area);
                    }
                    McpManagerView::ServerMenu { server_idx, action_idx } => {
                        if let Some(server) = mcp_mgr.servers.get(*server_idx) {
                            let actions = mcp_actions_for(server);
                            let overlay_height = (actions.len() as u16 + 4)
                                .max(6)
                                .min(area.height.saturating_sub(4));
                            let y = (area.height.saturating_sub(overlay_height)) / 2;
                            let overlay_area = ratatui::layout::Rect::new(x, y, overlay_width, overlay_height);
                            frame.render_widget(ratatui::widgets::Clear, overlay_area);

                            let status_line = Line::from(vec![
                                Span::styled("  State: ", Style::default().fg(t.muted)),
                                Span::styled(server.state.clone(), Style::default().fg(t.fg)),
                            ]);

                            let action_items: Vec<ListItem<'_>> = actions.iter().enumerate()
                                .map(|(i, act)| {
                                    let label = match *act {
                                        "reconnect" => "  Reconnect",
                                        "disable" => "  Disable",
                                        "enable" => "  Enable",
                                        "tools" => "  View Tools",
                                        "back" => "  Back",
                                        _ => act,
                                    };
                                    let style = if i == *action_idx {
                                        Style::default().fg(Color::Black).bg(t.accent).add_modifier(Modifier::BOLD)
                                    } else {
                                        Style::default().fg(t.fg)
                                    };
                                    ListItem::new(Line::from(Span::styled(label, style)))
                                })
                                .collect();

                            let mut all_items = vec![
                                ListItem::new(status_line),
                                ListItem::new(Line::from("")),
                            ];
                            all_items.extend(action_items);

                            let list = List::new(all_items)
                                .block(
                                    Block::default()
                                        .borders(Borders::ALL)
                                        .title(format!(" {} ", server.name))
                                        .border_style(Style::default().fg(t.accent))
                                );
                            frame.render_widget(list, overlay_area);
                        }
                    }
                    McpManagerView::ToolList { server_name, tools, scroll } => {
                            let visible = (area.height.saturating_sub(6)) as usize;
                            let overlay_height = (visible as u16 + 4).min(area.height.saturating_sub(4)).max(6);
                            let y = (area.height.saturating_sub(overlay_height)) / 2;
                            let overlay_area = ratatui::layout::Rect::new(x, y, overlay_width, overlay_height);
                            frame.render_widget(ratatui::widgets::Clear, overlay_area);

                            let items: Vec<ListItem<'_>> = if tools.is_empty() {
                                vec![ListItem::new(Line::from(Span::styled(
                                    "  (no tools)", Style::default().fg(t.muted)
                                )))]
                            } else {
                                tools.iter().skip(*scroll).take(visible)
                                    .map(|name| ListItem::new(Line::from(vec![
                                        Span::styled("  • ", Style::default().fg(t.accent)),
                                        Span::styled(name.clone(), Style::default().fg(t.fg)),
                                    ])))
                                    .collect()
                            };

                            let title = format!(" {} — tools (↑↓ scroll, Esc back) ", server_name);
                            let list = List::new(items)
                                .block(Block::default()
                                    .borders(Borders::ALL)
                                    .title(title)
                                    .border_style(Style::default().fg(t.accent)));
                            frame.render_widget(list, overlay_area);
                    }
                }
            }
        })?;

        // Handle events: use shorter poll when animation is active
        let timeout = if app.input.ultrathink.active || app.thinking.active {
            std::time::Duration::from_millis(80) // 12.5fps — smooth for bounce cycle
        } else {
            std::time::Duration::from_millis(250) // 4fps — poll returns immediately on events
        };

        // Check for agent events (non-blocking)
        while let Ok(tui_event) = event_rx.try_recv() {
            match tui_event {
                TuiEvent::TextDelta(text) => app.on_text_delta(&text),
                TuiEvent::ThinkingDelta(text) => app.on_thinking_delta(&text),
                TuiEvent::ToolStart { name, id } => app.on_tool_start(&name, &id),
                TuiEvent::ToolComplete { name, success } => {
                    app.on_tool_complete(&name, success);
                }
                TuiEvent::TurnComplete {
                    input_tokens,
                    output_tokens,
                } => {
                    app.on_turn_complete();
                    // Anthropic pricing: $3/MTok input, $15/MTok output
                    app.status.cost +=
                        (input_tokens as f64 * 3.0 + output_tokens as f64 * 15.0) / 1_000_000.0;
                    // Drain any input queued during generation
                    let queued: Vec<String> = app.pending_input.drain(..).collect();
                    for text in queued {
                        let _ = input_tx.send(text).await;
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
                TuiEvent::Done => {
                    app.should_quit = true;
                }
            }
        }

        if app.should_quit {
            break;
        }

        // Check for keyboard input; tick animations on timeout
        if event::poll(timeout)? {
            match event::read()? {
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
                                continue;
                            }
                            KeyCode::Down => {
                                if let Some(ref mut picker) = app.session_picker {
                                    if picker.selected + 1 < picker.sessions.len() {
                                        picker.selected += 1;
                                    } else {
                                        picker.selected = 0;
                                    }
                                }
                                continue;
                            }
                            KeyCode::Enter => {
                                if let Some(picker) = app.session_picker.take()
                                    && let Some(s) = picker.sessions.get(picker.selected)
                                {
                                    let _ =
                                        input_tx.send(format!("__resume_session__ {}", s.id)).await;
                                }
                                continue;
                            }
                            KeyCode::Esc => {
                                app.session_picker = None;
                                continue;
                            }
                            _ => continue, // swallow other keys
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
                                            let count =
                                                mcp_action_count(mgr.servers.get(*server_idx));
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
                                continue;
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
                                continue;
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
                                continue;
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
                                continue;
                            }
                            _ => continue, // swallow other keys while overlay is up
                        }
                    }
                    // Handle permission prompt — y/n/Enter/Esc
                    if app.permission_prompt.is_some() {
                        match key.code {
                            KeyCode::Char('y') | KeyCode::Char('Y') | KeyCode::Enter => {
                                let tool = app.permission_prompt.take().unwrap_or_default();
                                if let Some(ref tx) = permission_tx {
                                    let _ = tx.send(true).await;
                                }
                                app.output.append_line(&format!("[{tool}: approved]"));
                                continue;
                            }
                            KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => {
                                let tool = app.permission_prompt.take().unwrap_or_default();
                                if let Some(ref tx) = permission_tx {
                                    let _ = tx.send(false).await;
                                }
                                app.output.append_line(&format!("[{tool}: denied]"));
                                continue;
                            }
                            _ => continue, // swallow other keys during permission prompt
                        }
                    }
                    // Dismiss /btw overlay on any of Esc/Enter/Space
                    if app.btw_overlay.is_some() {
                        match key.code {
                            KeyCode::Esc | KeyCode::Enter | KeyCode::Char(' ') => {
                                app.btw_overlay = None;
                                continue;
                            }
                            _ => continue, // swallow all other keys while overlay is up
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
                        continue;
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
                        // Ctrl+V = voice hotkey; dispatch respects
                        // config.voice.toggle_mode (TASK-WIRE-007/009).
                        KeyEvent {
                            code: KeyCode::Char('v'),
                            modifiers: KeyModifiers::CONTROL,
                            ..
                        } => {
                            crate::voice::pipeline::fire_trigger_for_hotkey();
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
                            app.output.scroll_offset = u16::MAX; // will be clamped by effective_scroll
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
                                // Double-Esc within 500ms cancels generation
                                let now = std::time::Instant::now();
                                if let Some(last) = app.last_esc
                                    && now.duration_since(last).as_millis() < 500
                                {
                                    app.is_generating = false;
                                    app.active_tool = None;
                                    app.output.append_line("[interrupted]");
                                    app.last_esc = None;
                                    continue;
                                }
                                app.last_esc = Some(now);
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
                            let _ = input_tx.send(format!("/permissions {next}")).await;
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
                                    continue;
                                }
                            }
                            let text = app.submit_input();
                            if !text.is_empty() {
                                // /btw is ALWAYS immediate — never queued
                                if text.starts_with("/btw ") {
                                    if let Some(ref btw) = btw_tx {
                                        let question = text
                                            .strip_prefix("/btw ")
                                            .unwrap_or("")
                                            .trim()
                                            .to_string();
                                        if !question.is_empty() {
                                            let _ = btw.send(question).await;
                                        }
                                    } else {
                                        let _ = input_tx.send(text).await;
                                    }
                                } else if app.is_generating {
                                    // Queue non-btw input to send after current turn completes
                                    app.pending_input.push(text);
                                    app.output
                                        .append_line("[queued — will send after current turn]");
                                } else {
                                    let _ = input_tx.send(text).await;
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
                _ => {} // Resize and other events
            }
        } else {
            // No key event — tick animations
            app.input.ultrathink.tick();
            app.thinking.tick_thinking();
        }
    }

    // Restore terminal
    io::stdout().execute(DisableMouseCapture)?;
    terminal::disable_raw_mode()?;
    io::stdout().execute(LeaveAlternateScreen)?;

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
        app.on_tool_complete("Read", true);
        assert!(app.active_tool.is_none());
        // Successful tool calls do NOT append to output (no noise)
        assert!(app.output.all_lines().is_empty());
    }

    #[test]
    fn app_tool_failure_shows_in_output() {
        let mut app = App::new();
        app.on_tool_start("Bash", "tool-456");
        app.on_tool_complete("Bash", false);
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
