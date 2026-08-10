//! Issue #156 — standing gate against test fixtures that build filesystem
//! paths under a hardcoded `/tmp/`.
//!
//! ## The pathology
//!
//! Test fixtures across this workspace opened their sqlite stores at
//! `format!("/tmp/{prefix}-{uuid}.db")`. On Linux that is merely untidy —
//! nothing deletes the file, but `/tmp` is swept by the OS. On Windows
//! `/tmp` is not a temp directory at all: it is a root-relative path that
//! resolves against the *current drive*, so the stores landed in `F:\tmp\`
//! and stayed there. Measured 2026-08-09: **32,758 orphaned files, 0.6 GB,
//! accumulated over eight days** — one `.db` plus one
//! `.archon-cozo-write.lock` per guarded store per test run.
//!
//! The fix is `tempfile::TempDir`, owned by a guard that also owns the
//! database handle (`crate::command::test_db::TestDb` and its per-crate
//! siblings), so the directory cannot be removed while the store is open and
//! is always removed afterwards.
//!
//! ## What this gate flags — and what it deliberately does not
//!
//! A blanket ban on the substring `/tmp/` would be useless noise: the
//! workspace has ~250 occurrences, and the overwhelming majority are
//! **opaque identifiers** that never reach the filesystem —
//! `create_session("/tmp/proj", ..)`, `{"file_path": "/tmp/x"}`,
//! `PathBuf::from("/tmp/project")` fed to a pure path-computation function.
//! Flagging those would train everyone to add whitelist entries.
//!
//! So the gate pairs the literal with evidence that the path is *used to
//! create something*: within [`WINDOW`] lines of a `/tmp/` string literal,
//! one of [`FILESYSTEM_MARKERS`] must appear. `Uuid::new_v4` earns its place
//! in that list because a per-call unique path is the signature of a real
//! file — you do not randomise the name of a string you never open.
//!
//! On the tree as of this commit that rule fires on exactly the 18 sites
//! that opened a database and on nothing else; [`detector`] below pins both
//! directions against the real before/after snippets.
//!
//! Comment lines are skipped, so prose describing the banned pattern (this
//! file, and the `test_support` module docs) does not trip the gate.

use std::fs;
use std::path::{Path, PathBuf};

/// How many lines either side of the literal count as "the same statement".
///
/// Three covers the widest real offender, a `format!` split across four
/// lines by rustfmt:
///
/// ```text
/// let path = format!(
///     "/tmp/test-user-correction-event-{}.db",
///     uuid::Uuid::new_v4()
/// );
/// ```
const WINDOW: usize = 3;

/// Tokens that mean "this path is about to become a real file or directory".
const FILESYSTEM_MARKERS: &[&str] = &[
    "Uuid::new_v4",
    "DbInstance::new",
    "open_sqlite",
    "File::create",
    "fs::write",
    "create_dir",
    "OpenOptions",
];

/// Tokens that mean "this literal is the *default* when directory lookup
/// fails" — the production half of issue #156.
///
/// `dirs::home_dir().unwrap_or_else(|| PathBuf::from("/tmp"))` never fires on
/// a healthy machine, which is exactly why it survived: it is a root the
/// process then `create_dir_all`s into. `unwrap_or_default` is deliberately
/// not matched — `unwrap_or(` requires the open paren so the two do not
/// collide.
const FALLBACK_MARKERS: &[&str] = &["unwrap_or_else", "unwrap_or("];

fn worktree_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// Source roots to walk: the legacy worktree-root tree plus every crate's
/// `src`, `tests` and `examples`. `target/` is never entered because it is
/// not one of these roots.
fn scan_roots(root: &Path) -> Vec<PathBuf> {
    let mut roots = vec![root.join("src"), root.join("tests")];
    if let Ok(entries) = fs::read_dir(root.join("crates")) {
        for entry in entries.flatten() {
            let krate = entry.path();
            if !krate.is_dir() {
                continue;
            }
            for sub in ["src", "tests", "examples"] {
                roots.push(krate.join(sub));
            }
        }
    }
    roots
}

fn collect_rs_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let entries = match fs::read_dir(dir) {
        Ok(entries) => entries,
        // A missing `tests/` or `examples/` is normal, not a failure.
        Err(_) => return,
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let file_type = match entry.file_type() {
            Ok(file_type) => file_type,
            Err(_) => continue,
        };
        if file_type.is_symlink() {
            continue;
        }
        if file_type.is_dir() {
            collect_rs_files(&path, out);
        } else if path.extension().and_then(|ext| ext.to_str()) == Some("rs") {
            out.push(path);
        }
    }
}

/// Line numbers (1-based) where `literal` sits within [`WINDOW`] lines of one
/// of `markers` — close enough to count as the same statement.
///
/// Comment lines never carry the literal *or* the marker: prose about the
/// pattern is documentation, not a regression.
fn lines_pairing(contents: &str, literal: &str, markers: &[&str]) -> Vec<usize> {
    let lines: Vec<&str> = contents.lines().collect();
    let is_comment = |line: &str| line.trim_start().starts_with("//");

    let mut offenders = Vec::new();
    for (index, line) in lines.iter().enumerate() {
        if is_comment(line) || !line.contains(literal) {
            continue;
        }
        let low = index.saturating_sub(WINDOW);
        let high = (index + WINDOW).min(lines.len() - 1);
        let window_has_marker = lines[low..=high]
            .iter()
            .filter(|candidate| !is_comment(candidate))
            .any(|candidate| markers.iter().any(|marker| candidate.contains(marker)));
        if window_has_marker {
            offenders.push(index + 1);
        }
    }
    offenders
}

/// A `/tmp/…` literal that is about to become a real file or directory.
fn offending_lines(contents: &str) -> Vec<usize> {
    lines_pairing(contents, "/tmp/", FILESYSTEM_MARKERS)
}

/// A bare `"/tmp"` literal used as the fallback root when a directory lookup
/// fails. The closing quote is part of the pattern, so `"/tmp/project"` and
/// the other opaque identifiers are not candidates.
fn offending_fallback_lines(contents: &str) -> Vec<usize> {
    lines_pairing(contents, "\"/tmp\"", FALLBACK_MARKERS)
}

/// Every `.rs` file under the scan roots, paired with its worktree-relative
/// path. The gate's own source is excluded: it quotes both halves of every
/// pattern as data.
fn source_files(root: &Path) -> Vec<(String, String)> {
    let this_file = Path::new(file!())
        .file_name()
        .map(|name| name.to_os_string())
        .expect("gate has a file name");

    let mut files = Vec::new();
    for scan_root in scan_roots(root) {
        collect_rs_files(&scan_root, &mut files);
    }
    assert!(
        files.len() > 100,
        "walk found only {} files — the scan roots are wrong, which would \
         make this gate pass vacuously",
        files.len()
    );

    files
        .into_iter()
        .filter(|file| file.file_name() != Some(this_file.as_os_str()))
        .filter_map(|file| {
            let contents = fs::read_to_string(&file).ok()?;
            let relative = file
                .strip_prefix(root)
                .unwrap_or(&file)
                .to_string_lossy()
                .replace('\\', "/");
            Some((relative, contents))
        })
        .collect()
}

#[test]
fn no_source_file_builds_a_filesystem_path_under_hardcoded_tmp() {
    let root = worktree_root();

    let mut offenders: Vec<String> = Vec::new();
    for (relative, contents) in source_files(&root) {
        for line in offending_lines(&contents) {
            offenders.push(format!(
                "HARDCODED /tmp/ PATH (issue #156): {relative}:{line}"
            ));
        }
    }

    if !offenders.is_empty() {
        offenders.sort();
        let joined = offenders.join("\n");
        panic!(
            "\n{n} site(s) build a filesystem path under a hardcoded `/tmp/`. \
             On Windows that resolves against the current drive root (`F:\\tmp\\`) \
             and nothing ever deletes it. Use `tempfile::tempdir()` held by a \
             guard that outlives the handle — see `src/command/test_db.rs` — or \
             an in-memory Cozo instance if the test needs no persistence.\n\n{joined}\n",
            n = offenders.len(),
        );
    }
}

/// The production half: a directory lookup that defaults to `/tmp` when it
/// fails. `archon-core`'s agent-memory root and `archon-tools`'
/// worktree data dir both did this, and both then created directories under
/// the result.
#[test]
fn no_source_file_falls_back_to_a_hardcoded_tmp_root() {
    let root = worktree_root();

    let mut offenders: Vec<String> = Vec::new();
    for (relative, contents) in source_files(&root) {
        for line in offending_fallback_lines(&contents) {
            offenders.push(format!(
                "HARDCODED /tmp FALLBACK (issue #156): {relative}:{line}"
            ));
        }
    }

    if !offenders.is_empty() {
        offenders.sort();
        let joined = offenders.join("\n");
        panic!(
            "\n{n} site(s) fall back to a hardcoded `/tmp` when a directory \
             lookup fails. On Windows that is the current drive's root, not a \
             temp directory. Use `std::env::temp_dir()`.\n\n{joined}\n",
            n = offenders.len(),
        );
    }
}

/// Both directions of the detector, pinned against real code from this
/// repository. Without these the gate could silently stop matching and no
/// one would notice until `F:\tmp\` refilled.
#[cfg(test)]
mod detector {
    use super::*;

    /// The exact fixture bodies this issue removed.
    #[test]
    fn flags_the_original_offenders() {
        let single_line = "    fn test_db() -> DbInstance {\n\
             \x20       let path = format!(\"/tmp/test-completion-store-{}.db\", uuid::Uuid::new_v4());\n\
             \x20       DbInstance::new(\"sqlite\", &path, \"\").unwrap()\n\
             \x20   }\n";
        assert_eq!(offending_lines(single_line), vec![2]);

        // rustfmt-split `format!` — the reason WINDOW is 3 and not 0.
        let wrapped = "fn test_event_db() -> Arc<DbInstance> {\n\
             \x20   let path = format!(\n\
             \x20       \"/tmp/test-user-correction-event-{}.db\",\n\
             \x20       uuid::Uuid::new_v4()\n\
             \x20   );\n\
             }\n";
        assert_eq!(offending_lines(wrapped), vec![3]);
    }

    /// Opaque identifiers that never reach the filesystem. These are the
    /// ~250 occurrences the gate must stay quiet about.
    #[test]
    fn ignores_opaque_identifiers() {
        let opaque = "    store.create_session(\"/tmp/proj\", Some(\"main\"), \"m1\").unwrap();\n\
             \x20   let input = serde_json::json!({\"file_path\": \"/tmp/x\"});\n\
             \x20   assert_eq!(target.unwrap(), PathBuf::from(\"/tmp/project\"));\n\
             \x20   let cwd = Path::new(\"/tmp/project\");\n";
        assert!(offending_lines(opaque).is_empty());
    }

    /// Prose describing the banned pattern is documentation, not a leak.
    #[test]
    fn ignores_comments() {
        let documented = "//! Stores used to open at `format!(\"/tmp/x-{}.db\", uuid::Uuid::new_v4())`.\n\
             // let path = format!(\"/tmp/legacy-{}.db\", uuid::Uuid::new_v4());\n";
        assert!(offending_lines(documented).is_empty());
    }

    /// A marker further away than WINDOW is not evidence of the same
    /// statement, so the gate does not reach across unrelated code.
    #[test]
    fn marker_outside_the_window_does_not_match() {
        let distant = "let label = \"/tmp/project\";\n\
             let a = 1;\n\
             let b = 2;\n\
             let c = 3;\n\
             let d = 4;\n\
             let id = uuid::Uuid::new_v4();\n";
        assert!(offending_lines(distant).is_empty());
    }

    /// The two production fallbacks this issue removed, verbatim.
    #[test]
    fn flags_the_original_fallback_roots() {
        let home_dir = "        AgentMemoryScope::User => dirs::home_dir()\n\
             \x20           .unwrap_or_else(|| PathBuf::from(\"/tmp\"))\n\
             \x20           .join(\".archon/agent-memory\")\n";
        assert_eq!(offending_fallback_lines(home_dir), vec![2]);

        let env_home =
            "    let home = std::env::var(\"HOME\").unwrap_or_else(|_| \"/tmp\".to_string());\n";
        assert_eq!(offending_fallback_lines(env_home), vec![1]);
    }

    /// The replacements must not re-trip the gate they were written for.
    #[test]
    fn accepts_the_portable_replacements() {
        let fixed = "        AgentMemoryScope::User => dirs::home_dir()\n\
             \x20           .unwrap_or_else(std::env::temp_dir)\n\
             \x20   let home = std::env::var_os(\"HOME\")\n\
             \x20       .map(PathBuf::from)\n\
             \x20       .unwrap_or_else(std::env::temp_dir);\n";
        assert!(offending_fallback_lines(fixed).is_empty());
    }

    /// `"/tmp"` as an opaque argument is not a fallback root, and
    /// `unwrap_or_default` is not `unwrap_or(`.
    #[test]
    fn fallback_detector_ignores_opaque_and_near_misses() {
        let opaque = "    let session = store.create_session(\"/tmp\", None, \"m\").unwrap();\n\
             \x20   let cwd = value.unwrap_or_default();\n\
             \x20   assert_eq!(request.cwd.as_deref(), Some(\"/tmp\"));\n";
        assert!(offending_fallback_lines(opaque).is_empty());

        // A longer path is a different literal — the closing quote matters.
        let longer = "    let root = maybe.unwrap_or_else(|| PathBuf::from(\"/tmp/project\"));\n";
        assert!(offending_fallback_lines(longer).is_empty());
    }
}
