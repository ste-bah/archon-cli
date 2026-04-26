//! TASK-#207 SLASH-FILES /files slash-command handler.
//!
//! `/files` opens a file-picker overlay rooted at the session's
//! `working_dir`. User navigates the tree (Up/Down + Enter on dirs
//! to descend + Backspace to ascend) and picks a file with Enter,
//! at which point the file's absolute path is injected into the
//! input buffer prefixed with `@` (a mention-like marker — same
//! convention as `/skills` injecting `/skill-name `, with `@` chosen
//! to avoid collision with the slash-command prefix).
//!
//! # Architecture (overlay command)
//!
//! Mirrors TUI-627 `/skills` exactly:
//!
//!   - New `FileEntry` DTO in `archon-tui::events` (+ re-export via
//!     `archon-tui::app`).
//!   - New `TuiEvent::ShowFilePicker { root, entries }` variant.
//!   - New `FilePicker` screen at
//!     `crates/archon-tui/src/screens/file_picker/{mod,walker,render}.rs`
//!     with `selected_index` + `select_next/prev` nav + `descend` /
//!     `ascend` for in-place directory navigation.
//!   - `App::file_picker: Option<FilePicker>` field.
//!   - Event-loop arm sets `app.file_picker = Some(FilePicker::new(...))`.
//!   - Input priority branch routes Up/Down/Enter/Backspace/Esc.
//!   - `DirWalker` trait seam — `RealDirWalker` calls
//!     `archon_tui::screens::file_picker::walker::read_dir_entries`;
//!     tests inject `MockDirWalker` that returns canned entries.
//!
//! # Why not snapshot-pattern
//!
//! `/files` looks like a snapshot candidate (read-only, async-able
//! fetch in the builder), but the directory walk is sync and fast
//! (single `read_dir`, no recursion, no I/O across futures). Doing
//! it in `execute()` directly via the `DirWalker` seam keeps the
//! handler self-contained without pulling in another snapshot field
//! on `CommandContext`.

use std::path::PathBuf;
use std::sync::Arc;

use archon_tui::app::{FileEntry, TuiEvent};

use crate::command::registry::{CommandContext, CommandHandler};

/// Seam — tests inject `MockDirWalker`, production uses
/// `RealDirWalker` which delegates to the screen module's
/// `read_dir_entries` helper.
pub(crate) trait DirWalker: Send + Sync {
    fn read_dir(&self, path: &std::path::Path) -> std::io::Result<Vec<FileEntry>>;
}

/// Default `DirWalker` impl — calls the screen module's
/// `read_dir_entries` (which filters dotfiles + build-artifact dirs
/// + non-regular files and sorts dirs-first alphabetically).
pub(crate) struct RealDirWalker;

impl DirWalker for RealDirWalker {
    fn read_dir(&self, path: &std::path::Path) -> std::io::Result<Vec<FileEntry>> {
        // The screen module's `FileEntry` re-exports the layer-0
        // `events::FileEntry`, so the walker's return type is
        // already `Vec<FileEntry>` — no conversion needed.
        archon_tui::screens::file_picker::walker::read_dir_entries(path)
    }
}

/// `/files` handler.
pub(crate) struct FilesHandler {
    walker: Arc<dyn DirWalker>,
}

impl FilesHandler {
    pub(crate) fn new() -> Self {
        Self {
            walker: Arc::new(RealDirWalker),
        }
    }

    #[cfg(test)]
    pub(crate) fn with_walker(walker: Arc<dyn DirWalker>) -> Self {
        Self { walker }
    }
}

impl CommandHandler for FilesHandler {
    fn execute(&self, ctx: &mut CommandContext, _args: &[String]) -> anyhow::Result<()> {
        let working_dir: PathBuf = ctx
            .working_dir
            .as_ref()
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "FilesHandler invoked without working_dir populated \
                     — build_command_context bug"
                )
            })?
            .clone();

        let entries = self
            .walker
            .read_dir(&working_dir)
            .map_err(|e| anyhow::anyhow!("/files failed to read {}: {e}", working_dir.display()))?;

        ctx.emit(TuiEvent::ShowFilePicker {
            root: working_dir,
            entries,
        });
        Ok(())
    }

    fn description(&self) -> &str {
        "Browse and select files from the working directory"
    }

    fn aliases(&self) -> &'static [&'static str] {
        &[]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::command::test_support::*;

    struct MockDirWalker {
        result: std::sync::Mutex<Option<std::io::Result<Vec<FileEntry>>>>,
    }

    impl MockDirWalker {
        fn ok(entries: Vec<FileEntry>) -> Self {
            Self {
                result: std::sync::Mutex::new(Some(Ok(entries))),
            }
        }
        fn err(kind: std::io::ErrorKind, msg: &str) -> Self {
            Self {
                result: std::sync::Mutex::new(Some(Err(std::io::Error::new(
                    kind,
                    msg.to_string(),
                )))),
            }
        }
    }

    impl DirWalker for MockDirWalker {
        fn read_dir(&self, _path: &std::path::Path) -> std::io::Result<Vec<FileEntry>> {
            // Single-shot — `take()` so a second call would surface a
            // misuse rather than silently succeed.
            self.result
                .lock()
                .unwrap()
                .take()
                .unwrap_or_else(|| Ok(Vec::new()))
        }
    }

    fn entry(name: &str, is_dir: bool) -> FileEntry {
        FileEntry {
            name: name.to_string(),
            path: PathBuf::from(format!("/tmp/{}", name)),
            is_dir,
        }
    }

    #[test]
    fn execute_without_working_dir_returns_err() {
        let handler = FilesHandler::with_walker(Arc::new(MockDirWalker::ok(Vec::new())));
        let (mut ctx, _rx) = make_bug_ctx();
        let result = handler.execute(&mut ctx, &[]);
        assert!(result.is_err());
        let msg = format!("{:#}", result.unwrap_err()).to_lowercase();
        assert!(
            msg.contains("working_dir"),
            "error must reference working_dir; got: {}",
            msg
        );
    }

    #[test]
    fn execute_with_working_dir_emits_show_file_picker() {
        let entries = vec![entry("src", true), entry("Cargo.toml", false)];
        let handler = FilesHandler::with_walker(Arc::new(MockDirWalker::ok(entries.clone())));
        let (mut ctx, mut rx) = CtxBuilder::new()
            .with_working_dir(PathBuf::from("/tmp/myproject"))
            .build();
        handler.execute(&mut ctx, &[]).unwrap();
        let events = drain_tui_events(&mut rx);
        assert_eq!(events.len(), 1);
        match &events[0] {
            TuiEvent::ShowFilePicker { root, entries: e } => {
                assert_eq!(root, &PathBuf::from("/tmp/myproject"));
                assert_eq!(e.len(), 2);
                assert_eq!(e[0].name, "src");
                assert!(e[0].is_dir);
                assert_eq!(e[1].name, "Cargo.toml");
                assert!(!e[1].is_dir);
            }
            other => panic!("expected ShowFilePicker, got {:?}", other),
        }
    }

    #[test]
    fn execute_with_empty_dir_still_emits_event() {
        // Empty directory is a valid state — the overlay renders
        // "(empty directory)" and the user can Backspace out / Esc.
        let handler = FilesHandler::with_walker(Arc::new(MockDirWalker::ok(Vec::new())));
        let (mut ctx, mut rx) = CtxBuilder::new()
            .with_working_dir(PathBuf::from("/tmp/empty"))
            .build();
        handler.execute(&mut ctx, &[]).unwrap();
        let events = drain_tui_events(&mut rx);
        assert_eq!(events.len(), 1);
        match &events[0] {
            TuiEvent::ShowFilePicker { entries, .. } => {
                assert!(entries.is_empty());
            }
            other => panic!("expected ShowFilePicker, got {:?}", other),
        }
    }

    #[test]
    fn execute_with_walker_error_returns_err() {
        let handler = FilesHandler::with_walker(Arc::new(MockDirWalker::err(
            std::io::ErrorKind::PermissionDenied,
            "permission denied",
        )));
        let (mut ctx, _rx) = CtxBuilder::new()
            .with_working_dir(PathBuf::from("/root"))
            .build();
        let result = handler.execute(&mut ctx, &[]);
        assert!(result.is_err());
        let msg = format!("{:#}", result.unwrap_err()).to_lowercase();
        assert!(msg.contains("permission denied") || msg.contains("/files failed"));
    }

    #[test]
    fn description_and_aliases() {
        let h = FilesHandler::new();
        assert!(!h.description().is_empty());
        assert_eq!(h.aliases(), &[] as &[&'static str]);
    }

    #[test]
    #[ignore = "Gate 5 live smoke — exercises Registry dispatch via default_registry(), run via --ignored"]
    fn files_dispatches_via_registry() {
        // Gate-5 smoke: Registry::get("files") must return Some;
        // dispatching against a tempdir working_dir must emit a
        // ShowFilePicker event with the tempdir as root.
        use crate::command::registry::default_registry;

        let registry = default_registry();
        let handler = registry
            .get("files")
            .expect("files must be registered in default_registry()");

        let pid = std::process::id();
        let tmp = std::env::temp_dir().join(format!("files_smoke_{pid}"));
        let _ = std::fs::create_dir_all(&tmp);
        let (mut ctx, mut rx) = CtxBuilder::new().with_working_dir(tmp.clone()).build();
        let result = handler.execute(&mut ctx, &[]);
        let _ = std::fs::remove_dir_all(&tmp);

        result.unwrap();
        let events = drain_tui_events(&mut rx);
        assert_eq!(events.len(), 1);
        match &events[0] {
            TuiEvent::ShowFilePicker { root, .. } => {
                assert_eq!(root, &tmp);
            }
            other => panic!("expected ShowFilePicker, got {:?}", other),
        }
    }
}
