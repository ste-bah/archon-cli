//! Pane struct and content types for TASK-CLI-307 split pane layout.

/// Content type of a pane.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PaneContent {
    /// Main agent conversation view.
    Conversation,
    /// Log output (tracing events, debug info).
    Log,
    /// Tool call/result output.
    ToolOutput,
    /// Embedded terminal pane.
    Terminal,
}

/// A single pane in the split layout.
#[derive(Debug, Clone)]
pub struct Pane {
    /// Content type rendered in this pane.
    pub content: PaneContent,
    /// Scroll offset within this pane.
    pub scroll: u16,
    focused: bool,
}

impl Pane {
    /// Create a new pane with the given content type.
    pub fn new(content: PaneContent) -> Self {
        Self {
            content,
            scroll: 0,
            focused: false,
        }
    }

    /// Returns true if this pane has keyboard focus.
    pub fn is_focused(&self) -> bool {
        self.focused
    }

    /// Set the focus state for this pane.
    pub fn set_focused(&mut self, focused: bool) {
        self.focused = focused;
    }

    /// Scroll down by `n` lines.
    pub fn scroll_down(&mut self, n: u16) {
        self.scroll = self.scroll.saturating_add(n);
    }

    /// Scroll up by `n` lines.
    pub fn scroll_up(&mut self, n: u16) {
        self.scroll = self.scroll.saturating_sub(n);
    }
}
