//! Tasks overlay screen.
//! Layer 1 module — no imports from screens/ or app/.

use ratatui::Frame;
use ratatui::layout::{Constraint, Rect};
use ratatui::widgets::Row;

use crate::theme::Theme;
use crate::virtual_list::VirtualList;

/// Task identifier.
pub type TaskId = String;

/// A single row in the tasks overlay list.
#[derive(Debug, Clone)]
pub struct TaskRow {
    /// Unique identifier of the task.
    pub id: TaskId,
    /// Elapsed time in seconds.
    pub elapsed_secs: u64,
    /// Current status string (e.g. "running", "queued", "done").
    pub status: String,
}

/// Injectable source of task rows, and the way back to cancel one.
///
/// The overlay lives in `archon-tui`, which depends on `archon-tools` only as a
/// dev-dependency, so it cannot reach `TASK_MANAGER` directly. The binary owns
/// both and supplies the implementation; this trait is the whole seam.
///
/// `cancel_task` is on the same trait rather than a separate controller because
/// a list you cannot act on is what this overlay existed as for its whole life
/// before #189 — the read and the write belong together.
pub trait TaskStore: Send + Sync {
    fn list_tasks(&self) -> Vec<TaskRow>;

    /// Stop the identified task. `Err` carries a message fit for the status bar.
    fn cancel_task(&self, id: &TaskId) -> Result<(), String>;
}

/// Action emitted by the tasks overlay.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum TaskAction {
    #[default]
    None,
    CancelRequested(TaskId),
    InspectRequested(TaskId),
    RefreshRequested,
}

/// Tasks overlay state with virtualized scrolling.
#[derive(Debug)]
pub struct TaskOverlay {
    rows: VirtualList<TaskRow>,
    last_action: TaskAction,
}

impl TaskOverlay {
    /// Create a new TaskOverlay with the given initial rows.
    pub fn new(rows: Vec<TaskRow>) -> Self {
        Self {
            rows: VirtualList::new(rows, 10),
            last_action: TaskAction::None,
        }
    }

    /// Returns true if the overlay has no rows.
    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }

    crate::virtual_list::delegate_virtual_list!(rows, TaskRow);

    /// Returns the last emitted action.
    pub fn last_action(&self) -> TaskAction {
        self.last_action.clone()
    }

    /// Set task rows.
    pub fn set_rows(&mut self, rows: Vec<TaskRow>) {
        self.rows.set_items(rows);
    }

    /// Open (or re-open) the overlay with the given rows.
    pub fn open(&mut self, rows: Vec<TaskRow>) {
        self.rows.set_items(rows);
        self.last_action = TaskAction::None;
    }

    /// Request cancel for the currently selected task.
    pub fn cancel_selected(&mut self) {
        if let Some(row) = self.rows.selected() {
            self.last_action = TaskAction::CancelRequested(row.id.clone());
        }
    }

    /// Request inspect for the currently selected task.
    pub fn inspect_selected(&mut self) {
        if let Some(row) = self.rows.selected() {
            self.last_action = TaskAction::InspectRequested(row.id.clone());
        }
    }

    /// Request a refresh of the task list.
    pub fn refresh(&mut self) {
        self.last_action = TaskAction::RefreshRequested;
    }

    /// Clear the last emitted action.
    pub fn clear_action(&mut self) {
        self.last_action = TaskAction::None;
    }

    /// Draw the overlay into a centred rect inside `area`.
    ///
    /// `area` is the space available, not the space used — the overlay sizes
    /// itself to its rows and centres, matching the sibling pickers in
    /// `render/body/pickers.rs`. It previously rendered into the whole frame,
    /// which is why it covered the status bar and the input line.
    pub fn render(&self, f: &mut Frame, area: Rect, theme: &Theme) {
        const TITLE: &str = " Tasks — Up/Down select · x cancel · r refresh · Esc close ";

        // An empty frame is indistinguishable from a broken one. The no-store
        // case already says so in words; so should this.
        if self.rows.is_empty() {
            crate::overlay::message(f, area, TITLE, "Nothing is running.", theme);
            return;
        }

        // rows + header + two border lines.
        let (overlay, block) =
            crate::overlay::open(f, area, self.rows.len() as u16 + 3, TITLE, theme);

        let widths = [
            Constraint::Percentage(30),
            Constraint::Percentage(15),
            Constraint::Percentage(55),
        ];

        let rows: Vec<Row> = self
            .rows
            .items()
            .iter()
            .map(|r| {
                Row::new([
                    r.id.clone(),
                    format_elapsed(r.elapsed_secs),
                    r.status.clone(),
                ])
                .style(crate::overlay::body_style(theme))
            })
            .collect();

        // The selection has to reach the renderer or `move_up`/`move_down`
        // change an index nothing draws — which is what shipped: a list whose
        // cursor was invisible, so it looked like the keys did nothing.
        crate::overlay::render_table(
            f,
            overlay,
            block,
            Row::new(["ID", "Elapsed", "Status"]),
            rows,
            &widths,
            self.rows.selected_index(),
            theme,
        );
    }
}

impl Default for TaskOverlay {
    fn default() -> Self {
        Self::new(Vec::new())
    }
}

/// Format elapsed seconds as a human-readable string.
fn format_elapsed(secs: u64) -> String {
    let minutes = secs / 60;
    let seconds = secs % 60;
    if minutes > 0 {
        format!("{}m {}s", minutes, seconds)
    } else {
        format!("{}s", seconds)
    }
}

#[cfg(test)]
mod render_tests {
    //! Assertions about the drawn frame.
    //!
    //! The overlay shipped in #189 Phase 9 with a correct state machine and a
    //! render that drew none of it: no `Clear`, so the screen behind showed
    //! through; `render_widget` rather than `render_stateful_widget`, so the
    //! selection was invisible and Up/Down looked dead; the whole frame as its
    //! area, so it covered the status bar; and nothing at all when idle.
    //!
    //! Every existing test passed throughout, because they all assert on the
    //! model — that `move_up` changes an index, that `cancel_selected_task`
    //! returns the id it acted on. None of them looked at a buffer. These do.

    use super::*;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    fn rows() -> Vec<TaskRow> {
        vec![
            TaskRow {
                id: "task-alpha".into(),
                elapsed_secs: 5,
                status: "running".into(),
            },
            TaskRow {
                id: "task-beta".into(),
                elapsed_secs: 65,
                status: "running".into(),
            },
        ]
    }

    fn draw(overlay: &TaskOverlay) -> Terminal<TestBackend> {
        let mut terminal =
            Terminal::new(TestBackend::new(96, 24)).expect("build TestBackend terminal");
        terminal
            .draw(|frame| overlay.render(frame, frame.area(), &crate::theme::dark_theme()))
            .expect("draw tasks overlay");
        terminal
    }

    fn text(terminal: &Terminal<TestBackend>) -> String {
        let buffer = terminal.backend().buffer();
        let area = buffer.area;
        let mut out = String::new();
        for y in 0..area.height {
            for x in 0..area.width {
                out.push_str(buffer[(x, y)].symbol());
            }
            out.push('\n');
        }
        out
    }

    #[test]
    fn rows_and_their_elapsed_times_are_drawn() {
        let rendered = text(&draw(&TaskOverlay::new(rows())));
        assert!(rendered.contains("task-alpha"), "{rendered}");
        assert!(rendered.contains("task-beta"), "{rendered}");
        assert!(
            rendered.contains("1m 5s"),
            "elapsed not formatted: {rendered}"
        );
        assert!(rendered.contains("Status"), "header missing: {rendered}");
    }

    /// The defect that made it look broken: the selected row must be visibly
    /// distinct, which means the selection has to reach the renderer.
    #[test]
    fn the_selected_row_is_highlighted_and_moves_with_the_keys() {
        let mut overlay = TaskOverlay::new(rows());
        let terminal = draw(&overlay);
        let buffer = terminal.backend().buffer().clone();

        // Sample the cell the row's own text occupies. The overlay is centred,
        // so a fixed column lands in the blank margin outside it and reports
        // the same style for every row.
        let styles_of = |buffer: &ratatui::buffer::Buffer, needle: &str| {
            let area = buffer.area;
            for y in 0..area.height {
                let line: String = (0..area.width)
                    .map(|x| buffer[(x, y)].symbol().to_string())
                    .collect();
                if let Some(column) = line.find(needle) {
                    return Some(buffer[(column as u16, y)].style());
                }
            }
            None
        };

        let first = styles_of(&buffer, "task-alpha").expect("first row drawn");
        let second = styles_of(&buffer, "task-beta").expect("second row drawn");
        assert_ne!(
            first, second,
            "selected and unselected rows render identically, so the cursor is invisible"
        );

        overlay.move_down();
        let moved = draw(&overlay);
        let moved_buffer = moved.backend().buffer().clone();
        let now_first = styles_of(&moved_buffer, "task-alpha").expect("first row still drawn");
        assert_ne!(
            first, now_first,
            "moving the selection changed nothing on screen"
        );
    }

    /// An empty frame is indistinguishable from a broken widget.
    #[test]
    fn an_idle_session_is_told_so_in_words() {
        let rendered = text(&draw(&TaskOverlay::default()));
        assert!(
            rendered.contains("Nothing is running"),
            "idle overlay drew no explanation: {rendered}"
        );
    }

    /// It used to render into `frame.area()`, covering the status bar and the
    /// input line. It must leave the edges of the frame alone.
    #[test]
    fn the_overlay_does_not_cover_the_whole_frame() {
        let terminal = draw(&TaskOverlay::new(rows()));
        let buffer = terminal.backend().buffer();
        let area = buffer.area;

        let bottom: String = (0..area.width)
            .map(|x| buffer[(x, area.height - 1)].symbol().to_string())
            .collect();
        assert!(
            bottom.trim().is_empty(),
            "overlay painted the last row of the frame: {bottom:?}"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(id: &str, elapsed_secs: u64, status: &str) -> TaskRow {
        TaskRow {
            id: id.to_string(),
            elapsed_secs,
            status: status.to_string(),
        }
    }

    #[test]
    fn new_overlay_empty() {
        let overlay = TaskOverlay::new(vec![]);
        assert!(overlay.is_empty());
        assert_eq!(overlay.last_action(), TaskAction::None);
    }

    #[test]
    fn open_resets_action() {
        let mut overlay = TaskOverlay::new(vec![]);
        overlay.last_action = TaskAction::CancelRequested("x".into());
        overlay.open(vec![row("1", 10, "running")]);
        assert_eq!(overlay.last_action(), TaskAction::None);
    }

    #[test]
    fn set_rows_updates_list() {
        let mut overlay = TaskOverlay::new(vec![]);
        overlay.set_rows(vec![row("1", 10, "running"), row("2", 20, "queued")]);
        assert_eq!(overlay.len(), 2);
    }

    #[test]
    fn cancel_selected_emits_action() {
        let mut overlay = TaskOverlay::new(vec![
            row("task-1", 10, "running"),
            row("task-2", 20, "running"),
        ]);
        overlay.move_down();
        overlay.cancel_selected();
        assert_eq!(
            overlay.last_action(),
            TaskAction::CancelRequested("task-2".into())
        );
    }

    #[test]
    fn inspect_selected_emits_action() {
        let mut overlay = TaskOverlay::new(vec![row("task-1", 10, "running")]);
        overlay.inspect_selected();
        assert_eq!(
            overlay.last_action(),
            TaskAction::InspectRequested("task-1".into())
        );
    }

    #[test]
    fn cursor_wraps() {
        let mut overlay = TaskOverlay::new(vec![row("1", 10, "running"), row("2", 20, "running")]);
        assert_eq!(overlay.selected_index(), 0);
        overlay.move_down();
        assert_eq!(overlay.selected_index(), 1);
        overlay.move_down();
        assert_eq!(overlay.selected_index(), 0); // wrap
    }

    #[test]
    fn clear_action_resets() {
        let mut overlay = TaskOverlay::new(vec![row("1", 10, "running")]);
        overlay.cancel_selected();
        overlay.clear_action();
        assert_eq!(overlay.last_action(), TaskAction::None);
    }

    #[test]
    fn refresh_emits_refresh_requested() {
        let mut overlay = TaskOverlay::new(vec![row("task-1", 120, "running")]);
        overlay.refresh();
        assert_eq!(overlay.last_action(), TaskAction::RefreshRequested);
    }

    #[test]
    fn open_resets_cursor() {
        let mut overlay = TaskOverlay::new(vec![row("1", 10, "running"), row("2", 20, "running")]);
        overlay.move_down();
        overlay.open(vec![row("task-1", 10, "running")]);
        assert_eq!(overlay.selected_index(), 0);
    }

    /// The overlay reads its rows through `TaskStore`, so the trait is exercised
    /// here rather than only by the production implementation.
    #[test]
    fn rows_can_be_sourced_through_the_store_trait() {
        struct FixedStore(Vec<TaskRow>);
        impl TaskStore for FixedStore {
            fn list_tasks(&self) -> Vec<TaskRow> {
                self.0.clone()
            }

            fn cancel_task(&self, _id: &TaskId) -> Result<(), String> {
                Ok(())
            }
        }

        let store = FixedStore(vec![
            row("task-1", 10, "running"),
            row("task-2", 20, "done"),
        ]);
        let overlay = TaskOverlay::new(store.list_tasks());
        assert_eq!(overlay.len(), 2);
    }

    #[test]
    fn format_elapsed_shows_minutes_and_seconds() {
        assert_eq!(format_elapsed(0), "0s");
        assert_eq!(format_elapsed(5), "5s");
        assert_eq!(format_elapsed(60), "1m 0s");
        assert_eq!(format_elapsed(65), "1m 5s");
        assert_eq!(format_elapsed(125), "2m 5s");
    }
}
