//! In-TUI split pane layout system.
//!
//! Provides persistent side-by-side or stacked panes within the TUI,
//! independent of the tmux-based TerminalPanel.

use ratatui::layout::{Constraint, Direction, Layout, Rect};

// ---------------------------------------------------------------------------
// Pane content types
// ---------------------------------------------------------------------------

/// What a pane displays.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PaneContent {
    /// Main conversation output (the default view).
    Conversation,
    /// Tool output from a specific tool invocation.
    ToolOutput { tool_id: String },
    /// File preview (read-only display of a file).
    FilePreview { path: String },
    /// Subagent output stream.
    Agent { agent_id: String },
}

impl std::fmt::Display for PaneContent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Conversation => write!(f, "Conversation"),
            Self::ToolOutput { tool_id } => {
                write!(f, "Tool: {}", &tool_id[..8.min(tool_id.len())])
            }
            Self::FilePreview { path } => {
                let name = std::path::Path::new(path)
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or(path);
                write!(f, "File: {name}")
            }
            Self::Agent { agent_id } => {
                write!(f, "Agent: {}", &agent_id[..8.min(agent_id.len())])
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Pane layout
// ---------------------------------------------------------------------------

/// How the terminal area is divided.
#[derive(Debug, Clone, PartialEq, Default)]
pub enum PaneLayout {
    /// Single pane (default). No split.
    #[default]
    Single,
    /// Horizontal split: left | right. Ratio is the left pane's percentage (10-90).
    HorizontalSplit { ratio: u16 },
    /// Vertical split: top / bottom. Ratio is the top pane's percentage (10-90).
    VerticalSplit { ratio: u16 },
}

impl PaneLayout {
    /// Whether we're in a split view.
    pub fn is_split(&self) -> bool {
        !matches!(self, Self::Single)
    }

    /// Compute the ratatui Rect areas for the two panes (or one pane if Single).
    ///
    /// Returns `(primary_area, secondary_area)`. In Single mode, secondary is None.
    pub fn split_area(&self, area: Rect) -> (Rect, Option<Rect>) {
        match self {
            Self::Single => (area, None),
            Self::HorizontalSplit { ratio } => {
                let r = (*ratio).clamp(10, 90);
                let chunks = Layout::default()
                    .direction(Direction::Horizontal)
                    .constraints([Constraint::Percentage(r), Constraint::Percentage(100 - r)])
                    .split(area);
                (chunks[0], Some(chunks[1]))
            }
            Self::VerticalSplit { ratio } => {
                let r = (*ratio).clamp(10, 90);
                let chunks = Layout::default()
                    .direction(Direction::Vertical)
                    .constraints([Constraint::Percentage(r), Constraint::Percentage(100 - r)])
                    .split(area);
                (chunks[0], Some(chunks[1]))
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Pane state
// ---------------------------------------------------------------------------

/// State for a single pane.
#[derive(Debug, Clone)]
pub struct PaneState {
    /// What this pane displays.
    pub content: PaneContent,
    /// Text content buffer for non-conversation panes.
    pub buffer: String,
    /// Vertical scroll offset.
    pub scroll_offset: u16,
}

impl PaneState {
    pub fn new(content: PaneContent) -> Self {
        Self {
            content,
            buffer: String::new(),
            scroll_offset: 0,
        }
    }

    pub fn conversation() -> Self {
        Self::new(PaneContent::Conversation)
    }

    pub fn file_preview(path: &str) -> Self {
        Self::new(PaneContent::FilePreview {
            path: path.to_string(),
        })
    }

    pub fn tool_output(tool_id: &str) -> Self {
        Self::new(PaneContent::ToolOutput {
            tool_id: tool_id.to_string(),
        })
    }

    pub fn agent(agent_id: &str) -> Self {
        Self::new(PaneContent::Agent {
            agent_id: agent_id.to_string(),
        })
    }

    /// Scroll up by `amount` lines.
    pub fn scroll_up(&mut self, amount: u16) {
        self.scroll_offset = self.scroll_offset.saturating_add(amount);
    }

    /// Scroll down by `amount` lines.
    pub fn scroll_down(&mut self, amount: u16) {
        self.scroll_offset = self.scroll_offset.saturating_sub(amount);
    }

    /// Append text to the buffer.
    pub fn append(&mut self, text: &str) {
        self.buffer.push_str(text);
    }

    /// Clear the buffer and reset scroll.
    pub fn clear(&mut self) {
        self.buffer.clear();
        self.scroll_offset = 0;
    }

    /// Title for the pane border.
    pub fn title(&self) -> String {
        self.content.to_string()
    }
}

// ---------------------------------------------------------------------------
// Split pane manager
// ---------------------------------------------------------------------------

/// Manages the split pane state for the TUI.
#[derive(Debug, Clone)]
pub struct SplitPaneManager {
    /// Current layout mode.
    pub layout: PaneLayout,
    /// Primary pane (always conversation in single mode).
    pub primary: PaneState,
    /// Secondary pane (only used when split).
    pub secondary: Option<PaneState>,
    /// Which pane has focus: 0 = primary, 1 = secondary.
    pub focus: usize,
}

impl Default for SplitPaneManager {
    fn default() -> Self {
        Self {
            layout: PaneLayout::Single,
            primary: PaneState::conversation(),
            secondary: None,
            focus: 0,
        }
    }
}

impl SplitPaneManager {
    pub fn new() -> Self {
        Self::default()
    }

    /// Toggle split on/off. When toggling on, creates a horizontal split at 50%.
    /// When toggling off, returns to single pane.
    pub fn toggle_split(&mut self) {
        if self.layout.is_split() {
            self.layout = PaneLayout::Single;
            self.secondary = None;
            self.focus = 0;
        } else {
            self.layout = PaneLayout::HorizontalSplit { ratio: 50 };
            self.secondary = Some(PaneState::new(PaneContent::ToolOutput {
                tool_id: String::new(),
            }));
            self.focus = 0;
        }
    }

    /// Switch between horizontal and vertical split.
    pub fn rotate_layout(&mut self) {
        self.layout = match &self.layout {
            PaneLayout::Single => PaneLayout::HorizontalSplit { ratio: 50 },
            PaneLayout::HorizontalSplit { ratio } => PaneLayout::VerticalSplit { ratio: *ratio },
            PaneLayout::VerticalSplit { .. } => PaneLayout::Single,
        };
        if matches!(self.layout, PaneLayout::Single) {
            self.secondary = None;
            self.focus = 0;
        } else if self.secondary.is_none() {
            self.secondary = Some(PaneState::new(PaneContent::ToolOutput {
                tool_id: String::new(),
            }));
        }
    }

    /// Switch focus between primary and secondary pane.
    pub fn switch_focus(&mut self) {
        if self.secondary.is_some() {
            self.focus = if self.focus == 0 { 1 } else { 0 };
        }
    }

    /// Get a reference to the currently focused pane.
    pub fn focused_pane(&self) -> &PaneState {
        if self.focus == 1 {
            self.secondary.as_ref().unwrap_or(&self.primary)
        } else {
            &self.primary
        }
    }

    /// Get a mutable reference to the currently focused pane.
    pub fn focused_pane_mut(&mut self) -> &mut PaneState {
        if self.focus == 1
            && let Some(ref mut sec) = self.secondary
        {
            return sec;
        }
        &mut self.primary
    }

    /// Set the content of the secondary pane.
    pub fn set_secondary_content(&mut self, content: PaneContent) {
        if let Some(ref mut sec) = self.secondary {
            sec.content = content;
            sec.buffer.clear();
            sec.scroll_offset = 0;
        }
    }

    /// Resize the split ratio. Delta is in percentage points (+/- 5).
    pub fn resize(&mut self, delta: i16) {
        match &mut self.layout {
            PaneLayout::HorizontalSplit { ratio } | PaneLayout::VerticalSplit { ratio } => {
                let new = (*ratio as i16 + delta).clamp(10, 90) as u16;
                *ratio = new;
            }
            PaneLayout::Single => {}
        }
    }

    /// Compute the layout areas for the given output area rect.
    pub fn compute_areas(&self, area: Rect) -> (Rect, Option<Rect>) {
        self.layout.split_area(area)
    }

    /// Whether the primary pane has focus.
    pub fn primary_focused(&self) -> bool {
        self.focus == 0
    }

    /// Whether a secondary pane exists and has focus.
    pub fn secondary_focused(&self) -> bool {
        self.focus == 1 && self.secondary.is_some()
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_is_single_pane() {
        let mgr = SplitPaneManager::new();
        assert!(!mgr.layout.is_split());
        assert!(mgr.secondary.is_none());
        assert_eq!(mgr.focus, 0);
    }

    #[test]
    fn toggle_split_creates_horizontal() {
        let mut mgr = SplitPaneManager::new();
        mgr.toggle_split();
        assert!(matches!(
            mgr.layout,
            PaneLayout::HorizontalSplit { ratio: 50 }
        ));
        assert!(mgr.secondary.is_some());
    }

    #[test]
    fn toggle_split_back_to_single() {
        let mut mgr = SplitPaneManager::new();
        mgr.toggle_split();
        mgr.toggle_split();
        assert!(matches!(mgr.layout, PaneLayout::Single));
        assert!(mgr.secondary.is_none());
        assert_eq!(mgr.focus, 0);
    }

    #[test]
    fn rotate_layout_cycles() {
        let mut mgr = SplitPaneManager::new();
        mgr.rotate_layout(); // Single -> Horizontal
        assert!(matches!(mgr.layout, PaneLayout::HorizontalSplit { .. }));
        mgr.rotate_layout(); // Horizontal -> Vertical
        assert!(matches!(mgr.layout, PaneLayout::VerticalSplit { .. }));
        mgr.rotate_layout(); // Vertical -> Single
        assert!(matches!(mgr.layout, PaneLayout::Single));
    }

    #[test]
    fn switch_focus_toggles() {
        let mut mgr = SplitPaneManager::new();
        mgr.toggle_split();
        assert_eq!(mgr.focus, 0);
        mgr.switch_focus();
        assert_eq!(mgr.focus, 1);
        mgr.switch_focus();
        assert_eq!(mgr.focus, 0);
    }

    #[test]
    fn switch_focus_noop_in_single() {
        let mut mgr = SplitPaneManager::new();
        mgr.switch_focus(); // no secondary, stays at 0
        assert_eq!(mgr.focus, 0);
    }

    #[test]
    fn resize_clamps() {
        let mut mgr = SplitPaneManager::new();
        mgr.toggle_split();
        mgr.resize(50); // 50 + 50 = 100, clamped to 90
        if let PaneLayout::HorizontalSplit { ratio } = mgr.layout {
            assert_eq!(ratio, 90);
        }
        mgr.resize(-100); // 90 - 100 = -10, clamped to 10
        if let PaneLayout::HorizontalSplit { ratio } = mgr.layout {
            assert_eq!(ratio, 10);
        }
    }

    #[test]
    fn resize_noop_in_single() {
        let mut mgr = SplitPaneManager::new();
        mgr.resize(10); // no effect
        assert!(matches!(mgr.layout, PaneLayout::Single));
    }

    #[test]
    fn split_area_single_returns_full() {
        let area = Rect::new(0, 0, 100, 50);
        let (primary, secondary) = PaneLayout::Single.split_area(area);
        assert_eq!(primary, area);
        assert!(secondary.is_none());
    }

    #[test]
    fn split_area_horizontal_divides() {
        let area = Rect::new(0, 0, 100, 50);
        let layout = PaneLayout::HorizontalSplit { ratio: 50 };
        let (primary, secondary) = layout.split_area(area);
        assert!(primary.width > 0);
        assert!(secondary.is_some());
        let sec = secondary.unwrap();
        assert!(sec.width > 0);
        assert_eq!(primary.width + sec.width, area.width);
    }

    #[test]
    fn split_area_vertical_divides() {
        let area = Rect::new(0, 0, 100, 50);
        let layout = PaneLayout::VerticalSplit { ratio: 60 };
        let (primary, secondary) = layout.split_area(area);
        assert!(primary.height > 0);
        let sec = secondary.unwrap();
        assert!(sec.height > 0);
        assert_eq!(primary.height + sec.height, area.height);
    }

    #[test]
    fn pane_state_scroll() {
        let mut pane = PaneState::conversation();
        pane.scroll_up(5);
        assert_eq!(pane.scroll_offset, 5);
        pane.scroll_down(3);
        assert_eq!(pane.scroll_offset, 2);
        pane.scroll_down(10); // saturating
        assert_eq!(pane.scroll_offset, 0);
    }

    #[test]
    fn pane_content_display() {
        assert_eq!(PaneContent::Conversation.to_string(), "Conversation");
        let fp = PaneContent::FilePreview {
            path: "/home/user/src/main.rs".to_string(),
        };
        assert_eq!(fp.to_string(), "File: main.rs");
    }

    #[test]
    fn set_secondary_content_clears_buffer() {
        let mut mgr = SplitPaneManager::new();
        mgr.toggle_split();
        if let Some(ref mut sec) = mgr.secondary {
            sec.append("old content");
        }
        mgr.set_secondary_content(PaneContent::FilePreview {
            path: "test.rs".to_string(),
        });
        assert_eq!(mgr.secondary.as_ref().unwrap().buffer, "");
        assert!(matches!(
            mgr.secondary.as_ref().unwrap().content,
            PaneContent::FilePreview { .. }
        ));
    }

    #[test]
    fn focused_pane_follows_focus() {
        let mut mgr = SplitPaneManager::new();
        mgr.toggle_split();

        assert!(matches!(
            mgr.focused_pane().content,
            PaneContent::Conversation
        ));
        mgr.switch_focus();
        assert!(matches!(
            mgr.focused_pane().content,
            PaneContent::ToolOutput { .. }
        ));
    }
}
