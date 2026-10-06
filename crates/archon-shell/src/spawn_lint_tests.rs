//! Workspace lint: non-test code builds child processes only through
//! `archon_shell::spawn` (Issue 340), so no child can inherit a descriptor
//! another thread had not yet made close-on-exec.
//!
//! What counts as test code: a file whose path names a test (`tests`,
//! `_tests`, `test_support`, `/tests/`, `/benches/`), the dev-only
//! `archon-test-support` crate, build scripts (they run in cargo, not in
//! archon), the listed fixtures, and any `#[cfg(test)]` item in another file.
//! A command type renamed on import keeps `Command::new(` visible only when
//! the new name ends in `Command` (`TokioCommand::new(`); any other rename,
//! plain or inside braces (`use std::process::{Command as C}`), is flagged
//! at the rename. A raw `libc::fork(` is flagged unless the function that
//! calls it also calls `inherit_only_stdio`.

use std::path::{Path, PathBuf};

#[path = "spawn_lint_lex.rs"]
mod lex;

/// The one file allowed to call `Command::new`.
const HELPER: &str = "crates/archon-shell/src/spawn.rs";

/// Test-only files whose names do not say so; each is mounted under
/// `#[cfg(test)]` by its parent.
const TEST_FIXTURES: &[&str] = &["src/command/workflow_live_v3_run_end_heal_fixture.rs"];

/// Spellings that build a child without the helper: `open`'s launchers
/// spawn with a `Command` of their own (use `open::commands` with
/// `run_first_launcher`).
const FORBIDDEN: &[&str] = &[
    "Command::new(",
    "open::that(",
    "open::that_detached(",
    "open::that_in_background(",
    "open::with(",
    "open::with_detached(",
    "open::with_in_background(",
];

/// A rename of a command type that would hide `Command::new(` from the lint,
/// in a plain `use` or one item of a braced group (which may span lines).
fn forbidden_call(code: &str) -> bool {
    FORBIDDEN.iter().any(|f| whole_call(code, f))
}

fn whole_call(code: &str, spelling: &str) -> bool {
    code.match_indices(spelling).any(|(at, _)| {
        code[..at]
            .chars()
            .next_back()
            .is_none_or(|c| !(c.is_alphanumeric() || c == '_'))
    })
}

fn alias_call(code: &str, source: &str) -> bool {
    source.match_indices("Command as ").any(|(at, _)| {
        if source[..at]
            .chars()
            .next_back()
            .is_some_and(|c| c.is_alphanumeric() || c == '_')
        {
            return false;
        }
        let alias: String = source[at + 11..]
            .chars()
            .take_while(|c| c.is_alphanumeric() || *c == '_')
            .collect();
        !alias.is_empty() && whole_call(code, &format!("{alias}::new("))
    })
}

fn hiding_rename(code: &str) -> bool {
    let mut rest = code;
    while let Some(at) = rest.find("Command as ") {
        let whole_word = rest[..at]
            .chars()
            .next_back()
            .is_none_or(|c| !(c.is_alphanumeric() || c == '_'));
        let name: String = rest[at + "Command as ".len()..]
            .chars()
            .take_while(|c| c.is_alphanumeric() || *c == '_')
            .collect();
        if whole_word && !name.ends_with("Command") {
            return true;
        }
        rest = &rest[at + 1..];
    }
    false
}

/// Whether the function around line `index` of `lines` (0-based) calls
/// `inherit_only_stdio`: from the nearest `fn` at or above it to the brace
/// that closes it.
fn function_sweeps(lines: &[&str], index: usize) -> bool {
    let is_fn = |line: &str| {
        let code = line.split("//").next().unwrap_or("");
        code.trim_start().starts_with("fn ") || code.contains(" fn ")
    };
    let Some(start) = (0..=index).rev().find(|&at| is_fn(lines[at])) else {
        return false;
    };
    let mut depth = 0i64;
    let mut opened = false;
    for line in &lines[start..] {
        if line
            .split("//")
            .next()
            .unwrap_or("")
            .contains("inherit_only_stdio(")
        {
            return true;
        }
        depth += brace_delta(line);
        opened |= line.contains('{');
        if opened && depth <= 0 {
            break;
        }
    }
    false
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
    line.bytes().filter(|c| *c == b'{').count() as i64
        - line.bytes().filter(|c| *c == b'}').count() as i64
}

/// The lines of `source` outside `#[cfg(test)]` items, numbered from 1.
fn production_lines(source: &str) -> Vec<(usize, &str)> {
    let mut kept = Vec::new();
    let masked = lex::code(source);
    let original: Vec<_> = source.lines().collect();
    let mut lines = masked.lines().enumerate();
    while let Some((index, line)) = lines.next() {
        if line.trim() != "#[cfg(test)]" {
            kept.push((index + 1, original[index]));
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

fn workspace_rust_files(root: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    rust_files(&root.join("src"), &mut files);
    if let Ok(crates) = std::fs::read_dir(root.join("crates")) {
        for krate in crates.flatten() {
            rust_files(&krate.path().join("src"), &mut files);
        }
    }
    rust_files(&root.join("vendor/portable-pty/src"), &mut files);
    files
}

fn violations() -> Vec<String> {
    let root = workspace_root();
    let files = workspace_rust_files(&root);
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
        found.extend(source_violations(&relative, &source));
    }
    found
}

fn source_violations(relative: &str, source: &str) -> Vec<String> {
    if relative == HELPER || is_test_path(relative) {
        return Vec::new();
    }
    let mut found = Vec::new();
    let masked = lex::code(source);
    let all_lines: Vec<&str> = masked.lines().collect();
    for (number, line) in production_lines(source) {
        let code = all_lines[number - 1];
        // A raw fork that execs must sweep before it does.
        let raw_fork = code.contains("libc::fork(") && !function_sweeps(&all_lines, number - 1);
        if forbidden_call(code) || alias_call(code, &masked) || hiding_rename(code) || raw_fork {
            found.push(format!("{relative}:{number}: {}", line.trim()));
        }
    }
    found
}

#[test]
fn non_test_code_builds_children_only_through_the_spawn_helper() {
    let found = violations();
    assert!(
        found.is_empty(),
        "build child processes with archon_shell::spawn::{{command, tokio_command, \
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
    assert!(hiding_rename(
        "use std::process::{Child, Command as C, Stdio};"
    ));
    assert!(hiding_rename("    Command as Spawn,"));
    assert!(!hiding_rename(
        "use x::{SubCommand as Sub, Command as BuildCommand};"
    ));
    assert!(
        FORBIDDEN
            .iter()
            .any(|f| "open::that_detached(&url)".contains(f))
    );
}

#[test]
fn a_raw_fork_must_sweep_in_the_same_function() {
    let lines: Vec<&str> = "fn sweeps() {
    libc::fork();
    inherit_only_stdio(c);
}
                            fn bare() {
    libc::fork();
}
"
    .lines()
    .collect();
    assert!(function_sweeps(&lines, 1));
    assert!(
        !function_sweeps(&lines, 5),
        "a sweep in another function counts"
    );
}

#[test]
fn unrelated_command_type_is_not_a_process_spawn() {
    assert!(
        [
            "FooCommand::new(1)",
            "JobCommand::new(1)",
            "BuildCommand::new(1)"
        ]
        .iter()
        .all(|code| !forbidden_call(code)),
        "an unrelated type is mistaken for std::process::Command"
    );
}

#[test]
fn raw_quote_in_test_does_not_hide_production_spawn() {
    let source = "#[cfg(test)]\nmod tests {\nlet s = r#\"\"{\"#;\n}\nCommand::new(\"git\");";
    assert!(production_lines(source).iter().any(|(n, _)| *n == 5));
}

#[test]
fn raw_brace_in_test_does_not_expose_test_spawn() {
    let source =
        "#[cfg(test)]\nmod tests {\nlet s = r#\"\"}\"#;\nCommand::new(\"git\");\n}\nfn after() {}";
    assert_eq!(
        production_lines(source)
            .iter()
            .map(|(n, _)| *n)
            .collect::<Vec<_>>(),
        vec![6]
    );
}

#[test]
fn multiline_raw_string_cannot_change_test_item_depth() {
    let source = "#[cfg(test)]\nmod tests {\nlet s = r##\"\n}\n\"##;\nCommand::new(\"git\");\n}\nfn after() {}";
    assert_eq!(
        production_lines(source)
            .iter()
            .map(|(n, _)| *n)
            .collect::<Vec<_>>(),
        vec![8]
    );
}

#[path = "spawn_lint_vendor_tests.rs"]
mod vendor_tests;
