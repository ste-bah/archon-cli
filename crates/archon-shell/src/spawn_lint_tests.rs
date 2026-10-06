//! Workspace lint: non-test code builds child processes only through
//! `archon_shell::spawn` (Issue 340), so no child can inherit a descriptor
//! another thread had not yet made close-on-exec.
//!
//! What counts as test code: a file whose path names a test (`tests`,
//! `_tests`, `test_support`, `/tests/`, `/benches/`), the dev-only
//! `archon-test-support` crate, build scripts (they run in cargo, not in
//! archon), the listed fixtures, and any `#[cfg(test)]` item in another file.
//! A command type renamed on import keeps `Command::new(` visible only when
//! the new name ends in `Command` (`TokioCommand::new(`); any other rename
//! (`use std::process::Command as C`) is flagged at the import. A raw
//! `libc::fork(` is flagged unless its file applies `inherit_only_stdio`.

use std::path::{Path, PathBuf};

/// The one file allowed to call `Command::new`.
const HELPER: &str = "crates/archon-shell/src/spawn.rs";

/// Test-only files whose names do not say so; each is mounted under
/// `#[cfg(test)]` by its parent.
const TEST_FIXTURES: &[&str] = &["src/command/workflow_live_v3_run_end_heal_fixture.rs"];

/// Spellings that build a child without the helper.
const FORBIDDEN: &[&str] = &["Command::new(", "open::that("];

/// A rename of a command type that would hide `Command::new(` from the lint.
fn hiding_rename(code: &str) -> bool {
    code.split("process::Command as ").skip(1).any(|rest| {
        let name: String = rest
            .chars()
            .take_while(|c| c.is_alphanumeric() || *c == '_')
            .collect();
        !name.ends_with("Command")
    })
}

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn is_test_path(relative: &str) -> bool {
    let name = relative.rsplit('/').next().unwrap_or(relative);
    let stem = name.trim_end_matches(".rs");
    relative.starts_with("crates/archon-test-support/")
        || relative.contains("/tests/")
        || relative.starts_with("tests/")
        || relative.contains("/benches/")
        || relative.split('/').any(|part| part.ends_with("_tests"))
        || stem == "tests"
        || stem.starts_with("tests_")
        || stem.contains("_tests")
        || stem.contains("test_support")
        || name == "build.rs"
        || TEST_FIXTURES.contains(&relative)
}

fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            rust_files(&path, out);
        } else if path.extension().is_some_and(|ext| ext == "rs") {
            out.push(path);
        }
    }
}

/// `{` minus `}` on a line, ignoring string and char literals and comments.
fn brace_delta(line: &str) -> i64 {
    let mut delta = 0;
    let mut chars = line.chars().peekable();
    let mut in_string = false;
    while let Some(c) = chars.next() {
        match c {
            '\\' if in_string => {
                chars.next();
            }
            '"' => in_string = !in_string,
            '/' if !in_string && chars.peek() == Some(&'/') => break,
            '\'' if !in_string => {
                // A char literal such as '{' or '\''; a lifetime has no
                // closing quote within three characters.
                let rest: String = chars.clone().take(3).collect();
                if let Some(end) = rest.find('\'') {
                    for _ in 0..=end {
                        chars.next();
                    }
                }
            }
            '{' if !in_string => delta += 1,
            '}' if !in_string => delta -= 1,
            _ => {}
        }
    }
    delta
}

/// The lines of `source` outside `#[cfg(test)]` items, numbered from 1.
fn production_lines(source: &str) -> Vec<(usize, &str)> {
    let mut kept = Vec::new();
    let mut lines = source.lines().enumerate();
    while let Some((index, line)) = lines.next() {
        if line.trim() != "#[cfg(test)]" {
            kept.push((index + 1, line));
            continue;
        }
        // Skip the gated item: further attributes, then either a one-line
        // item (`mod tests;`) or a braced block until it balances.
        let mut depth = 0i64;
        let mut opened = false;
        for (_, item) in lines.by_ref() {
            let trimmed = item.trim();
            if !opened && trimmed.starts_with("#[") {
                continue;
            }
            depth += brace_delta(item);
            opened |= item.contains('{');
            if (opened && depth <= 0) || (!opened && trimmed.ends_with(';')) {
                break;
            }
        }
    }
    kept
}

fn violations() -> Vec<String> {
    let root = workspace_root();
    let mut files = Vec::new();
    rust_files(&root.join("src"), &mut files);
    if let Ok(crates) = std::fs::read_dir(root.join("crates")) {
        for krate in crates.flatten() {
            rust_files(&krate.path().join("src"), &mut files);
        }
    }
    // A wrong root would scan nothing and pass.
    assert!(root.join(HELPER).is_file(), "workspace root not found");
    assert!(files.len() > 100, "only {} files scanned", files.len());
    let mut found = Vec::new();
    for path in files {
        let relative = path
            .strip_prefix(&root)
            .unwrap_or(&path)
            .to_string_lossy()
            .replace('\\', "/");
        if relative == HELPER || is_test_path(&relative) {
            continue;
        }
        let Ok(source) = std::fs::read_to_string(&path) else {
            continue;
        };
        for (number, line) in production_lines(&source) {
            let code = line.split("//").next().unwrap_or("");
            // A raw fork that execs must sweep before it does.
            let raw_fork = code.contains("libc::fork(") && !source.contains("inherit_only_stdio(");
            if FORBIDDEN.iter().any(|spelling| code.contains(spelling))
                || hiding_rename(code)
                || raw_fork
            {
                found.push(format!("{relative}:{number}: {}", line.trim()));
            }
        }
    }
    found
}

#[test]
fn non_test_code_builds_children_only_through_the_spawn_helper() {
    let found = violations();
    assert!(
        found.is_empty(),
        "build child processes with crate::spawn::{{command, tokio_command, \
         stdio_only}} so they inherit only stdio (Issue 340):\n{}",
        found.join("\n")
    );
}

#[test]
fn the_lint_sees_a_spawn_in_production_code_and_not_in_a_test_item() {
    let source = "fn run() {\n    let c = Command::new(\"git\");\n}\n\
                  #[cfg(test)]\nmod tests {\n    fn t() { let s = \"}\"; Command::new(\"x\"); }\n}\n\
                  fn after() {}\n";
    let lines = production_lines(source);
    let numbers: Vec<usize> = lines.iter().map(|(number, _)| *number).collect();
    assert_eq!(numbers, vec![1, 2, 3, 8], "{lines:?}");
    assert!(is_test_path("crates/x/src/foo_tests.rs"));
    assert!(is_test_path("crates/x/src/foo_tests/tree.rs"));
    assert!(!is_test_path("crates/x/src/test_baseline_run.rs"));
    assert!(hiding_rename("use std::process::Command as Cmd;"));
    assert!(!hiding_rename(
        "use tokio::process::Command as TokioCommand;"
    ));
}
