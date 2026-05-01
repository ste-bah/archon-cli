//! Generic virtualized scrolling list.
//! Renders only visible viewport slice of a potentially long list.
//! Layer 1 module — no imports from screens/ or app/.

/// VirtualList is a generic virtualized list that tracks selection
/// and viewport offset without rendering directly (rendering belongs
/// to the caller via `visible_items()`).
#[derive(Debug)]
pub struct VirtualList<T> {
    items: Vec<T>,
    selected: usize,
    viewport_offset: usize,
    viewport_height: usize,
}

impl<T> VirtualList<T> {
    pub fn new(items: Vec<T>, viewport_height: usize) -> Self {
        let selected = 0;
        Self {
            items,
            selected,
            viewport_offset: 0,
            viewport_height,
        }
    }

    pub fn with_selected(mut self, selected: usize) -> Self {
        if selected < self.items.len() {
            self.selected = selected;
            self.scroll_to_selected();
        }
        self
    }

    /// Items visible in the current viewport.
    pub fn visible_items(&self) -> &[T] {
        let end = (self.viewport_offset + self.viewport_height).min(self.items.len());
        if self.viewport_offset >= end {
            &[]
        } else {
            &self.items[self.viewport_offset..end]
        }
    }

    pub fn selected_index(&self) -> usize {
        self.selected
    }

    pub fn selected(&self) -> Option<&T> {
        self.items.get(self.selected)
    }

    pub fn move_up(&mut self) {
        if self.selected > 0 {
            self.selected -= 1;
            self.scroll_to_selected();
        } else {
            // Wrap to last item
            if !self.items.is_empty() {
                self.selected = self.items.len() - 1;
                self.scroll_to_selected();
            }
        }
    }

    pub fn move_down(&mut self) {
        if self.selected + 1 < self.items.len() {
            self.selected += 1;
            self.scroll_to_selected();
        } else {
            // Wrap to first item
            if !self.items.is_empty() {
                self.selected = 0;
                self.scroll_to_selected();
            }
        }
    }

    pub fn page_up(&mut self) {
        let new_selected = self.selected.saturating_sub(self.viewport_height);
        self.selected = new_selected;
        self.viewport_offset = self.selected;
    }

    pub fn page_down(&mut self) {
        let new_selected =
            (self.selected + self.viewport_height).min(self.items.len().saturating_sub(1));
        self.selected = new_selected;
        self.scroll_to_selected();
    }

    pub fn set_items(&mut self, items: Vec<T>) {
        let prev_len = self.items.len();
        self.items = items;
        // Clamp selected to valid range after items change
        if prev_len == 0 {
            self.selected = 0;
        } else if self.selected >= self.items.len() {
            self.selected = self.items.len().saturating_sub(1);
        }
        // Keep viewport_offset aligned with selected
        if self.selected < self.viewport_offset {
            self.viewport_offset = self.selected;
        }
        let bottom = self.viewport_offset + self.viewport_height;
        if self.selected >= bottom {
            self.viewport_offset = self
                .selected
                .saturating_sub(self.viewport_height.saturating_sub(1));
        }
    }

    pub fn len(&self) -> usize {
        self.items.len()
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    fn scroll_to_selected(&mut self) {
        // If selected is above viewport, scroll up
        if self.selected < self.viewport_offset {
            self.viewport_offset = self.selected;
        }
        // If selected is below viewport, scroll down
        let bottom = self.viewport_offset + self.viewport_height;
        if self.selected >= bottom {
            self.viewport_offset = self
                .selected
                .saturating_sub(self.viewport_height.saturating_sub(1));
        }
    }
}

impl<T> Default for VirtualList<T> {
    fn default() -> Self {
        Self {
            items: Vec::new(),
            selected: 0,
            viewport_offset: 0,
            viewport_height: 10,
        }
    }
}
