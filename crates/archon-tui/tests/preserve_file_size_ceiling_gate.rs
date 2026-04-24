//! TASK-TUI-903 / ERR-TUI-004 — 500-line ceiling preservation gate.
//!
//! Reference: SPEC-TUI-PRESERVE AC-PRESERVE-04, TC-TUI-PRESERVE-04
//! Reference: TECH-TUI-PRESERVE INV-PRESERVE-001
//!
//! ERR-TUI-004 was the pre-fix pathology where `src/main.rs` grew to
//! 5870 lines. SPEC-TUI-MODULARIZATION owns the positive refactor; this
//! file owns the **standing size gate** that prevents any future PR from
//! reintroducing a >=500-line .rs file in the TUI crate (or in the
//! legacy worktree-root `src/` tree that still carries transitional
//! code during the migration).
//!
//! ## Scope
//!
//! - Walks `crates/archon-tui/src/**/*.rs` (the TUI crate proper)
//! - Walks `<worktree_root>/src/**/*.rs` (legacy-root carryover — if the
//!   directory is absent the scan yields zero files, which is fine)
//!
//! ## Exclusions
//!
//! - Files named `build.rs` (cargo build scripts are not library code)
//! - Files matching `*.generated.rs` (auto-generated, size not policy-relevant)
//! - Files whose first non-shebang line contains `// @generated`
//!   (canonical Rust generated-file marker)
//!
//! ## Whitelist
//!
//! An override file `preserve_file_size_whitelist.txt` sits next to this
//! test. It is compiled in via `include_str!` so the gate has no runtime
//! file-resolution dependency on the whitelist. Entries are paths
//! relative to the worktree root, one per line, `#` comments and blank
//! lines allowed. Whitelist entries MUST carry a trailing
//! `# expires YYYY-MM-DD` comment; TASK-TUI-905 audits expiries each
//! stage. (This gate does not enforce the expiry format — that is a
//! separate lint; here we only consume the path portion before `#`.)
//!
//! ## Failure format
//!
//! Each offending file produces a line of the form
//!
//! ```text
//! FILE TOO LARGE (ERR-TUI-004): <path>:<N> lines (ceiling=500)
//! ```
//!
//! The test panics with the full collected list on failure.

use std::fs;
use std::path::{Path, PathBuf};

/// Inclusive ceiling: any file with >= this many lines fails the gate.
const LINE_CEILING: usize = 500;

/// Whitelist file, baked in at compile time.
const WHITELIST_RAW: &str = include_str!("preserve_file_size_whitelist.txt");

/// Resolve the worktree root from CARGO_MANIFEST_DIR.
///
/// A test binary compiled from `crates/archon-tui/tests/*.rs` has
/// `CARGO_MANIFEST_DIR = <worktree>/crates/archon-tui`. Jumping up two
/// levels lands on the worktree root.
fn worktree_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
}

/// Parse the whitelist into a set of worktree-root-relative path strings.
///
/// Lines are split at the first `#` (comment boundary), whitespace is
/// trimmed, blanks are skipped. Path separators are normalised to `/`
/// so the whitelist is identical on any platform.
fn parse_whitelist(raw: &str) -> Vec<String> {
    let mut out = Vec::new();
    for line in raw.lines() {
        let no_comment = match line.find('#') {
            Some(idx) => &line[..idx],
            None => line,
        };
        let trimmed = no_comment.trim();
        if trimmed.is_empty() {
            continue;
        }
        out.push(trimmed.replace('\\', "/"));
    }
    out
}

/// Recursively collect every `.rs` file under `dir`.
///
/// If `dir` does not exist, returns an empty Vec silently (spec:
/// "the scan still tries and finds zero files, no error"). Symlinks
/// are not followed to avoid walking out of the tree.
fn collect_rs_files(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    if !dir.exists() {
        return out;
    }
    walk(dir, &mut out);
    out
}

fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
    let entries = match fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let ft = match entry.file_type() {
            Ok(ft) => ft,
            Err(_) => continue,
        };
        if ft.is_symlink() {
            continue;
        }
        if ft.is_dir() {
            walk(&path, out);
            continue;
        }
        if ft.is_file() && path.extension().and_then(|s| s.to_str()) == Some("rs") {
            out.push(path);
        }
    }
}

/// Returns true if this file is excluded from the size policy.
///
/// Exclusions:
/// - file name is exactly `build.rs`
/// - file name ends with `.generated.rs`
/// - first non-shebang line contains `// @generated`
fn is_excluded(path: &Path, contents: &str) -> bool {
    if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
        if name == "build.rs" {
            return true;
        }
        if name.ends_with(".generated.rs") {
            return true;
        }
    }
    // Skip optional shebang, inspect the first real line for @generated.
    let first_real_line = contents
        .lines()
        .find(|l| !l.starts_with("#!"))
        .unwrap_or("");
    if first_real_line.contains("// @generated") {
        return true;
    }
    false
}

/// Count lines by `\n` terminator (spec).
///
/// This differs from `str::lines()` because the last line has no
/// trailing newline in most Rust files and we want the
/// `wc -l`-equivalent count (number of line terminators seen). A file
/// with N newline-terminated lines plus one unterminated trailing line
/// is reported as N+1 lines.
fn count_lines(contents: &str) -> usize {
    let newline_terminated = contents.bytes().filter(|&b| b == b'\n').count();
    let trailing = if contents.ends_with('\n') || contents.is_empty() {
        0
    } else {
        1
    };
    newline_terminated + trailing
}

/// Produce a worktree-root-relative path string with forward slashes.
fn relativise(root: &Path, file: &Path) -> String {
    let rel = file.strip_prefix(root).unwrap_or(file);
    rel.to_string_lossy().replace('\\', "/")
}

#[test]
fn preserve_file_size_ceiling_gate() {
    let root = worktree_root();
    let whitelist = parse_whitelist(WHITELIST_RAW);

    // Validation criterion 4: walk at least 2 source roots.
    let scan_roots: [PathBuf; 2] = [
        root.join("crates").join("archon-tui").join("src"),
        root.join("src"),
    ];

    let mut offenders: Vec<String> = Vec::new();

    for scan_root in &scan_roots {
        let files = collect_rs_files(scan_root);
        for file in files {
            let contents = match fs::read_to_string(&file) {
                Ok(c) => c,
                // Unreadable files are not offenders — skip silently.
                // If a file genuinely cannot be read, other tests will fail.
                Err(_) => continue,
            };
            if is_excluded(&file, &contents) {
                continue;
            }
            let line_count = count_lines(&contents);
            if line_count < LINE_CEILING {
                continue;
            }
            let rel = relativise(&root, &file);
            if whitelist.iter().any(|w| w == &rel) {
                continue;
            }
            offenders.push(format!(
                "FILE TOO LARGE (ERR-TUI-004): {rel}:{line_count} lines (ceiling={LINE_CEILING})"
            ));
        }
    }

    if !offenders.is_empty() {
        offenders.sort();
        let joined = offenders.join("\n");
        panic!(
            "\nTASK-TUI-903 / ERR-TUI-004 preservation gate failed. \
             {n} file(s) at or above the {ceiling}-line ceiling. \
             Split the file, or add it to \
             crates/archon-tui/tests/preserve_file_size_whitelist.txt \
             with a `# expires YYYY-MM-DD` comment (audited by TASK-TUI-905).\n\n{joined}\n",
            n = offenders.len(),
            ceiling = LINE_CEILING,
            joined = joined
        );
    }
}

// ---------------------------------------------------------------------------
// Unit tests for the gate's own helpers. These run alongside the main gate
// test; they are cheap and self-contained (no filesystem access) and
// guard against silent regressions in the line-counter or the whitelist
// parser, either of which would make the gate trivially pass.
// ---------------------------------------------------------------------------

#[cfg(test)]
mod helpers {
    use super::*;

    #[test]
    fn line_count_matches_wc_l_semantics() {
        assert_eq!(count_lines(""), 0);
        assert_eq!(count_lines("a\n"), 1);
        assert_eq!(count_lines("a\nb\n"), 2);
        // Unterminated trailing line still counts.
        assert_eq!(count_lines("a\nb"), 2);
        assert_eq!(count_lines("a"), 1);
    }

    #[test]
    fn whitelist_parser_strips_comments_and_blanks() {
        let raw = "# header\n\ncrates/foo/src/bar.rs # expires 2026-05-01\n   \n#another\n";
        let parsed = parse_whitelist(raw);
        assert_eq!(parsed, vec!["crates/foo/src/bar.rs".to_string()]);
    }

    #[test]
    fn exclusion_matches_build_and_generated() {
        use std::path::PathBuf;
        assert!(is_excluded(&PathBuf::from("crates/foo/build.rs"), ""));
        assert!(is_excluded(
            &PathBuf::from("crates/foo/src/api.generated.rs"),
            ""
        ));
        assert!(is_excluded(
            &PathBuf::from("crates/foo/src/x.rs"),
            "// @generated by prost\nmod y;\n"
        ));
        // Shebang is skipped when looking for the @generated marker.
        assert!(is_excluded(
            &PathBuf::from("crates/foo/src/x.rs"),
            "#!/usr/bin/env rustc\n// @generated\nmod y;\n"
        ));
        assert!(!is_excluded(
            &PathBuf::from("crates/foo/src/x.rs"),
            "mod y;\n"
        ));
    }
}
