//! Keeping the bytes a trimmed tool result left out (#189 Phase 1).
//!
//! A large tool result is replayed to the model as a head/tail excerpt with a
//! note saying how much was omitted. That is the right trade for the context
//! window, but the omitted region was previously unreachable: the note said
//! bytes were dropped and gave no way to read them, so recovering the middle of
//! a 200 KB grep meant running the grep again.
//!
//! This writes the whole result to `.archon/spill/<session>/<call>-<tool>.txt`
//! once, at ingest, and hands back a locator the truncation note can name. The
//! model then has a path it can `Read`.
//!
//! Two things this deliberately does not do:
//!
//! - It does not spill from `cap_tool_output_to_bytes`. That function is pure
//!   and is also used by compaction-internal trimming, where a file per trimmed
//!   message would be noise, and by the request projection, which runs on every
//!   request and would rewrite the same file over and over.
//! - It does not spill results that are already file-backed. Copying a `Read` of
//!   a path the model can re-read into a second file on disk buys nothing.

use std::path::{Path, PathBuf};

/// Where a tool result's full output was written.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpillLocator {
    /// Absolute path to the spilled output.
    pub path: PathBuf,
    /// Exact byte length written, so the note can be specific.
    pub bytes: usize,
}

impl SpillLocator {
    /// The sentence appended to a truncation note.
    #[must_use]
    pub fn note(&self) -> String {
        format!(
            " Full output: {} (read it if you need the omitted region).",
            self.path.display()
        )
    }
}

/// Tools whose results are already retrievable from where they came from.
///
/// `Read` and `NotebookRead` return the contents of a path the model still
/// holds; re-reading the original is strictly better than reading a stale copy,
/// because the file may have changed since.
const ALREADY_RETRIEVABLE: &[&str] = &["Read", "NotebookRead"];

/// Whether a result from this tool is worth spilling.
#[must_use]
pub fn is_spillable(tool_name: &str) -> bool {
    !ALREADY_RETRIEVABLE.contains(&tool_name)
}

/// Root of a working directory's spill storage.
#[must_use]
pub fn spill_root(working_dir: &Path) -> PathBuf {
    working_dir.join(".archon").join("spill")
}

/// Write `content` for one tool call and return where it went.
///
/// Failure is returned rather than propagated into the turn: a full disk or a
/// read-only checkout should cost the retrieval path, not the tool result. The
/// caller logs and carries on with an unlocated truncation note.
pub fn save(
    working_dir: &Path,
    session_id: &str,
    tool_name: &str,
    call_id: &str,
    content: &str,
) -> std::io::Result<SpillLocator> {
    let dir = spill_root(working_dir).join(sanitize(session_id));
    std::fs::create_dir_all(&dir)?;
    let path = dir.join(format!("{}-{}.txt", sanitize(call_id), sanitize(tool_name)));
    std::fs::write(&path, content)?;
    Ok(SpillLocator {
        path: std::fs::canonicalize(&path).unwrap_or(path),
        bytes: content.len(),
    })
}

/// Reduce an identifier to something safe to use as a path component.
///
/// Tool-call ids and tool names come from the provider, and a `/` or `..` in
/// one would place the write outside the spill directory. Anything that is not
/// clearly inert is replaced.
fn sanitize(raw: &str) -> String {
    let cleaned: String = raw
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect();
    if cleaned.is_empty() {
        "unnamed".to_string()
    } else {
        cleaned
    }
}

/// Delete session spill directories older than the retention window.
///
/// Called at session start. Returns how many directories were removed; errors
/// on individual entries are skipped rather than aborting the sweep, because
/// one unreadable directory should not leave every other one to accumulate.
pub fn prune(working_dir: &Path, retention: Option<std::time::Duration>) -> usize {
    prune_at(working_dir, retention, std::time::SystemTime::now())
}

/// [`prune`] with the clock supplied.
///
/// Split out so the sweep can be tested against a real directory tree without
/// either sleeping or reaching for a crate that can backdate an mtime.
fn prune_at(
    working_dir: &Path,
    retention: Option<std::time::Duration>,
    now: std::time::SystemTime,
) -> usize {
    let Some(retention) = retention else {
        return 0;
    };
    let root = spill_root(working_dir);
    let Ok(entries) = std::fs::read_dir(&root) else {
        return 0;
    };
    let mut removed = 0;
    for entry in entries.flatten() {
        if !entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
            continue;
        }
        let expired = entry
            .metadata()
            .and_then(|meta| meta.modified())
            .ok()
            .and_then(|modified| now.duration_since(modified).ok())
            .is_some_and(|age| age > retention);
        if expired && std::fs::remove_dir_all(entry.path()).is_ok() {
            removed += 1;
        }
    }
    removed
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn saved_output_is_byte_identical_to_the_original() {
        let dir = tempfile::tempdir().expect("tempdir");
        let content = format!("HEAD{}TAIL", "é".repeat(50_000));

        let locator = save(dir.path(), "sess-1", "Bash", "call-1", &content).expect("spill");

        assert_eq!(locator.bytes, content.len());
        assert_eq!(
            std::fs::read_to_string(&locator.path).expect("read back"),
            content,
            "the whole point is that the omitted region survives"
        );
    }

    #[test]
    fn the_note_names_the_path() {
        let dir = tempfile::tempdir().expect("tempdir");
        let locator = save(dir.path(), "s", "Bash", "c", "output").expect("spill");

        let note = locator.note();

        assert!(note.contains(&locator.path.display().to_string()), "{note}");
        assert!(note.contains("omitted region"), "{note}");
    }

    #[test]
    fn each_call_gets_its_own_file() {
        let dir = tempfile::tempdir().expect("tempdir");
        let first = save(dir.path(), "s", "Bash", "call-1", "one").expect("spill");
        let second = save(dir.path(), "s", "Bash", "call-2", "two").expect("spill");

        assert_ne!(first.path, second.path);
        assert_eq!(std::fs::read_to_string(&first.path).unwrap(), "one");
        assert_eq!(std::fs::read_to_string(&second.path).unwrap(), "two");
    }

    /// Ids come from the provider. A `..` in one must not walk out of the
    /// spill directory and write somewhere else on disk.
    #[test]
    fn a_traversing_id_cannot_escape_the_spill_directory() {
        let dir = tempfile::tempdir().expect("tempdir");

        let locator = save(dir.path(), "../../etc", "Bash", "../../../passwd", "x")
            .expect("spill still succeeds");

        let root = std::fs::canonicalize(spill_root(dir.path())).expect("canonical root");
        assert!(
            locator.path.starts_with(&root),
            "{} escaped {}",
            locator.path.display(),
            root.display()
        );
    }

    #[test]
    fn file_backed_results_are_not_spilled() {
        assert!(!is_spillable("Read"));
        assert!(!is_spillable("NotebookRead"));
        assert!(is_spillable("Bash"));
        assert!(is_spillable("Grep"));
    }

    const WEEK: std::time::Duration = std::time::Duration::from_secs(7 * 86_400);

    #[test]
    fn a_session_directory_past_the_window_is_removed() {
        let dir = tempfile::tempdir().expect("tempdir");
        let locator = save(dir.path(), "old-session", "Bash", "c", "drop").expect("spill");
        let a_month_on = std::time::SystemTime::now() + std::time::Duration::from_secs(30 * 86_400);

        let removed = prune_at(dir.path(), Some(WEEK), a_month_on);

        assert_eq!(removed, 1);
        assert!(!locator.path.exists());
    }

    /// The same fixture, viewed from the same later moment, survives a window
    /// wide enough to contain it — so the previous test is measuring age, not
    /// just deleting whatever it finds.
    #[test]
    fn a_session_directory_inside_the_window_is_kept() {
        let dir = tempfile::tempdir().expect("tempdir");
        let locator = save(dir.path(), "recent-session", "Bash", "c", "keep").expect("spill");
        let a_month_on = std::time::SystemTime::now() + std::time::Duration::from_secs(30 * 86_400);

        let removed = prune_at(
            dir.path(),
            Some(std::time::Duration::from_secs(60 * 86_400)),
            a_month_on,
        );

        assert_eq!(removed, 0);
        assert!(locator.path.exists());
    }

    #[test]
    fn pruning_is_a_no_op_when_retention_is_disabled() {
        let dir = tempfile::tempdir().expect("tempdir");
        let locator = save(dir.path(), "s", "Bash", "c", "keep").expect("spill");

        assert_eq!(prune(dir.path(), None), 0);
        assert!(locator.path.exists());
    }

    #[test]
    fn pruning_a_directory_that_was_never_created_is_harmless() {
        let dir = tempfile::tempdir().expect("tempdir");
        assert_eq!(
            prune(dir.path(), Some(std::time::Duration::from_secs(1))),
            0
        );
    }
}
