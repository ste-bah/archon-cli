//! Centered modal overlay stack for TUI overlays.
//! Layer 1 module — no imports from screens/ or app/.

use ratatui::{Frame, layout::Rect};
use std::vec::Vec;

/// Trait for renderable overlays.
pub trait OverlayRenderer: Send {
    fn render(&self, f: &mut Frame, area: Rect);
    fn name(&self) -> &str;
}

/// Stack of overlays. The top overlay is interactive; lower overlays
/// are dimmed but still visible.
pub struct OverlayStack {
    stack: Vec<Box<dyn OverlayRenderer>>,
}

impl OverlayStack {
    pub fn new() -> Self {
        Self { stack: Vec::new() }
    }

    pub fn is_empty(&self) -> bool {
        self.stack.is_empty()
    }

    pub fn len(&self) -> usize {
        self.stack.len()
    }

    pub fn push<R: OverlayRenderer + 'static>(&mut self, overlay: R) {
        self.stack.push(Box::new(overlay));
    }

    pub fn pop(&mut self) -> Option<Box<dyn OverlayRenderer>> {
        self.stack.pop()
    }

    /// Returns a reference to the top overlay without removing it.
    pub fn peek(&self) -> Option<&dyn OverlayRenderer> {
        self.stack.last().map(|b| b.as_ref())
    }

    /// Renders the top overlay only.
    pub fn render(&self, f: &mut Frame, area: Rect) {
        // If empty, nothing to render
        if self.stack.is_empty() {
            return;
        }
        // For a simple single-overlay implementation, just render top.
        // Multi-overlay dimming can be added later.
        if let Some(top) = self.stack.last() {
            top.render(f, area);
        }
    }
}

impl Default for OverlayStack {
    fn default() -> Self {
        Self::new()
    }
}
