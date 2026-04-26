//! TASK-#207 SLASH-FILES file-picker overlay (screen module).
//!
//! Pre-planned 3-file sub-module split (per Steven's reinforcement
//! from Phase 4 kick-off — avoid #204-class file-size cleanup
//! follow-up by splitting BEFORE implementation):
//!
//!   - `mod.rs`     — `FileEntry` type + `FilePicker` struct +
//!     navigation surface (`new`, `select_next/prev`, `selected`,
//!     `descend`, `ascend`).
//!   - `walker.rs`  — `read_dir_entries` helper: enumerate directory
//!     contents, filter hidden + common build-artifact dirs, sort
//!     dirs-first then alphabetical.
//!   - `render.rs`  — drawing: centered overlay, breadcrumb header,
//!     scrollable list, dir/file badge prefix.
//!
//! The picker is FLAT — at any given moment it lists ONE directory's
//! contents (the canonical fzf/skim file-picker UX). Up/Down moves
//! the cursor; Enter on a directory descends into it (re-walks);
//! Enter on a file injects `@<absolute-path> ` into the input buffer
//! and closes; Backspace ascends to the parent directory (clamped to
//! the original `working_dir` root); Esc closes. Recursive walks +
//! gitignore-aware filtering are deferred to a follow-up — for now
//! we hardcode-skip `.git`, `target`, `node_modules`, and dotfiles
//! at every level, which covers the 95% case.

pub mod render;
pub mod walker;

use std::path::{Path, PathBuf};

// Reuse the layer-0 `FileEntry` so that `TuiEvent::ShowFilePicker {
// entries: Vec<FileEntry> }` and `FilePicker::new(_, entries:
// Vec<FileEntry>)` agree on a single type. Defining a parallel
// `FileEntry` inside this module would force every emit-site to
// `into_iter().map(...)` between two structurally-identical types
// for no benefit.
pub use crate::events::FileEntry;

/// File-picker overlay state.
///
/// `root` is the directory the picker was opened in (the original
/// `working_dir`). Ascent is clamped here — Backspace at the root
/// is a no-op rather than an escape from the workspace.
pub struct FilePicker {
    /// Original working directory the picker was opened in. Ascent
    /// is clamped to this prefix.
    pub root: PathBuf,
    /// Currently-displayed directory. Equal to `root` initially;
    /// changes on `descend` / `ascend`.
    pub current_dir: PathBuf,
    /// Entries currently listed (sorted dirs-first, alphabetical).
    pub entries: Vec<FileEntry>,
    /// Index into `entries` of the highlighted row.
    pub selected_index: usize,
}

impl FilePicker {
    /// Construct a picker rooted at `root` with a pre-walked initial
    /// listing. The slash handler walks the directory once before
    /// emitting `TuiEvent::ShowFilePicker` to avoid doing I/O inside
    /// the event-loop handler arm.
    pub fn new(root: PathBuf, entries: Vec<FileEntry>) -> Self {
        Self {
            current_dir: root.clone(),
            root,
            entries,
            selected_index: 0,
        }
    }

    pub fn select_next(&mut self) {
        if self.entries.is_empty() {
            return;
        }
        self.selected_index = (self.selected_index + 1) % self.entries.len();
    }

    pub fn select_prev(&mut self) {
        if self.entries.is_empty() {
            return;
        }
        self.selected_index = if self.selected_index == 0 {
            self.entries.len() - 1
        } else {
            self.selected_index - 1
        };
    }

    pub fn selected(&self) -> Option<&FileEntry> {
        self.entries.get(self.selected_index)
    }

    /// If the selected entry is a directory, walk it and replace
    /// `current_dir` + `entries`. Returns `true` on a successful
    /// descent, `false` if the selected entry is a file or the walk
    /// fails. Selected index resets to 0 on success.
    pub fn descend(&mut self) -> bool {
        let target = match self.selected() {
            Some(e) if e.is_dir => e.path.clone(),
            _ => return false,
        };
        match walker::read_dir_entries(&target) {
            Ok(new_entries) => {
                self.current_dir = target;
                self.entries = new_entries;
                self.selected_index = 0;
                true
            }
            Err(_) => false,
        }
    }

    /// Walk the parent of `current_dir` and replace state, clamped
    /// to `root`. Returns `true` on a successful ascent, `false` if
    /// already at `root` or the walk fails.
    pub fn ascend(&mut self) -> bool {
        if self.current_dir == self.root {
            return false;
        }
        let parent = match self.current_dir.parent() {
            Some(p) => p.to_path_buf(),
            None => return false,
        };
        // Refuse to ascend above `root` even if parent traversal
        // would technically allow it (defensive against symlinks
        // and ../ navigation).
        if !is_within_or_equal(&parent, &self.root) {
            return false;
        }
        match walker::read_dir_entries(&parent) {
            Ok(new_entries) => {
                self.current_dir = parent;
                self.entries = new_entries;
                self.selected_index = 0;
                true
            }
            Err(_) => false,
        }
    }

    /// Display-only — `current_dir` rendered relative to `root` (or
    /// the absolute path if `current_dir` somehow drifts outside the
    /// root, which `ascend` should prevent).
    pub fn breadcrumb(&self) -> String {
        match self.current_dir.strip_prefix(&self.root) {
            Ok(rel) if rel.as_os_str().is_empty() => ".".to_string(),
            Ok(rel) => format!("./{}", rel.display()),
            Err(_) => self.current_dir.display().to_string(),
        }
    }
}

/// Return `true` if `child` equals `parent` or is a descendant of it.
/// Used by `ascend` to clamp navigation to the picker's root.
fn is_within_or_equal(child: &Path, parent: &Path) -> bool {
    child == parent || child.starts_with(parent)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn entry(name: &str, is_dir: bool) -> FileEntry {
        FileEntry {
            name: name.to_string(),
            path: PathBuf::from(format!("/tmp/{}", name)),
            is_dir,
        }
    }

    #[test]
    fn new_starts_at_zero() {
        let p = FilePicker::new(
            PathBuf::from("/tmp"),
            vec![entry("a", false), entry("b", false)],
        );
        assert_eq!(p.selected_index, 0);
        assert_eq!(p.current_dir, p.root);
    }

    #[test]
    fn select_next_wraps() {
        let mut p =
            FilePicker::new(PathBuf::from("/tmp"), vec![entry("a", false), entry("b", false)]);
        p.select_next();
        assert_eq!(p.selected_index, 1);
        p.select_next();
        assert_eq!(p.selected_index, 0);
    }

    #[test]
    fn select_prev_wraps_at_start() {
        let mut p =
            FilePicker::new(PathBuf::from("/tmp"), vec![entry("a", false), entry("b", false)]);
        p.select_prev();
        assert_eq!(p.selected_index, 1);
    }

    #[test]
    fn empty_list_noop() {
        let mut p = FilePicker::new(PathBuf::from("/tmp"), vec![]);
        p.select_next();
        p.select_prev();
        assert_eq!(p.selected_index, 0);
        assert!(p.selected().is_none());
        assert!(!p.descend());
        assert!(!p.ascend());
    }

    #[test]
    fn descend_into_file_is_noop() {
        let mut p = FilePicker::new(
            PathBuf::from("/tmp"),
            vec![entry("a-file", false)],
        );
        assert!(!p.descend());
    }

    #[test]
    fn ascend_at_root_is_noop() {
        let mut p =
            FilePicker::new(PathBuf::from("/tmp"), vec![entry("a", false)]);
        assert!(!p.ascend());
    }

    #[test]
    fn ascend_then_descend_round_trip() {
        // Build a real tempdir so descend/ascend can actually call
        // read_dir + return Ok. /tmp/file_picker_round_trip_<pid>/
        // {a/, b.txt}.
        let pid = std::process::id();
        let root = std::env::temp_dir().join(format!("file_picker_round_trip_{pid}"));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("a")).unwrap();
        fs::write(root.join("a").join("inner.txt"), b"x").unwrap();
        fs::write(root.join("b.txt"), b"y").unwrap();

        let initial = walker::read_dir_entries(&root).unwrap();
        let mut p = FilePicker::new(root.clone(), initial);

        // Find the dir entry index.
        let dir_idx = p.entries.iter().position(|e| e.is_dir && e.name == "a");
        let dir_idx = dir_idx.expect("a/ must be in listing");
        p.selected_index = dir_idx;

        assert!(p.descend(), "descend into a/ must succeed");
        assert_eq!(p.current_dir, root.join("a"));
        // Inside a/: should see inner.txt.
        assert!(p.entries.iter().any(|e| e.name == "inner.txt"));

        assert!(p.ascend(), "ascend back to root must succeed");
        assert_eq!(p.current_dir, root);

        // Cleanup.
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn breadcrumb_root_is_dot() {
        let p = FilePicker::new(PathBuf::from("/tmp/myproject"), vec![]);
        assert_eq!(p.breadcrumb(), ".");
    }

    #[test]
    fn breadcrumb_subdir_is_relative() {
        let mut p =
            FilePicker::new(PathBuf::from("/tmp/myproject"), vec![]);
        p.current_dir = PathBuf::from("/tmp/myproject/src/foo");
        let crumb = p.breadcrumb();
        assert!(
            crumb == "./src/foo",
            "expected `./src/foo`, got `{}`",
            crumb
        );
    }
}
