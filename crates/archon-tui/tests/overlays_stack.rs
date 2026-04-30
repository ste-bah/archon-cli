use archon_tui::overlays::{OverlayRenderer, OverlayStack};
use ratatui::{Frame, layout::Rect};

struct DummyOverlay(&'static str);

impl OverlayRenderer for DummyOverlay {
    fn render(&self, _f: &mut Frame, _area: Rect) {}
    fn name(&self) -> &str {
        self.0
    }
}

#[test]
fn push_pop_is_empty() {
    let mut stack = OverlayStack::new();
    assert!(stack.is_empty());
    stack.push(DummyOverlay("test"));
    assert!(!stack.is_empty());
    assert_eq!(stack.len(), 1);
    let popped = stack.pop();
    assert!(popped.is_some());
    assert!(stack.is_empty());
}
