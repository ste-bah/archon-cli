//! The source files a task's declared focused-test commands resolve to,
//! made writable for the task that declares them (Issue-71).
//!
//! # The gap this closes
//!
//! A task's write scope is its declared deliverables, enforced at write
//! time (Issue-64). But a task's declared focused tests routinely live in a
//! test module that is NOT a deliverable — the task body tells the coder to
//! add tests there, the filter names the module, and nothing declares its
//! file. Live: a coder told to write its tests in
//! `<crate>/src/<mod>/<mod>_tests/<case>.rs` was refused the write, because
//! the module was not among the task's deliverables, and told to log a
//! residual gap instead. The declared filter and the declared targets
//! disagreed, and the guard sided with the targets.
//!
//! # What is widened
//!
//! Each declared command is read for its selection: a positional filter
//! `a::b::c` (the first non-option token after `cargo test` / `cargo nextest
//! run`, before any `--`), or an integration test named by `--test <name>`.
//! `--test name` resolves to `<pkg>/tests/name.rs` (or `tests/name/main.rs`)
//! and `<pkg>/tests/name/`. Any command that is not a cargo test run (`cargo
//! clippy`, `cargo fmt`, a script) widens nothing.
//!
//! A module filter is a SUBSTRING match to cargo and nextest, and task
//! authors write it that way: live, a task declared a two-segment filter
//! `store_tests::cases` for a module three levels deep,
//! `store::store_tests::cases` (Issue-72). So the filter's
//! segments are resolved under the package's `src`, longest prefix first
//! (a trailing test-function name simply fails to match and the module
//! part is tried next), and at each length two shapes are tried in order:
//!
//! 1. the EXACT walk from the src root, by the same machinery that owns a
//!    failing test ([`super::test_baseline_owner`]): `a/b/c.rs`,
//!    `a/b/c/mod.rs`, the `#[path]` sibling `a/b_tests.rs` for a `tests`
//!    segment;
//! 2. the SUFFIX search: every module file in the src tree whose module
//!    path ends with those segments (`…/a/b/c.rs`, `…/a/b/c/mod.rs`, and
//!    `…/a/b_tests.rs` read as `…::a::b::tests`). Exactly one hit resolves;
//!    several is ambiguous and widens nothing, and the branch is told which
//!    files tied so the filter can be narrowed.
//!
//! Longest first means a full-segment suffix hit beats a shallower exact
//! parent; exact first at each length means a real parent module beats a
//! same-depth stray elsewhere in the tree. A filter whose leaf module does
//! not exist yet therefore still lands on its parent module file, which is
//! where its `mod` line must be added. A prefix that is nothing but `tests`
//! segments is never suffix-searched: it names no module of its own. The
//! file's module directory (`a/b/c/` for `a/b/c.rs` or `a/b/c/mod.rs`) is
//! widened with it, so new sibling test files can be created. A crate-root
//! hit, a filter no package resolves, and a filter more than one package
//! resolves (ambiguous) widen nothing.
//!
//! # Whose file it stays
//!
//! The baseline's ownership rule is mirrored: a resolved file that ANOTHER
//! task in the universe declares is that task's and is not widened here —
//! its tests are that task's to write. A file no task declares, or this
//! task's own, is widened. The module directory is widened only when the
//! file was, and only when no other task declares a path within it.

use std::path::Path;

use crate::task_universe::WorkflowV2TaskUniverse;

use super::test_baseline_owner::{self, Ownership};

/// What a cargo test command selects.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Selection {
    /// A positional module-path filter, split on `::`; a bare name is one
    /// segment.
    Module(Vec<String>),
    /// `--test <name>`: an integration test binary.
    IntegrationTest(String),
    /// Nothing this module can resolve to a file.
    None,
}

/// The file a command resolves to and the module directory beside it, both
/// repo-relative; the directory carries no trailing slash.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct FocusedTestTarget {
    pub file: String,
    pub dir: String,
}

/// What a command's selection resolves to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Resolution {
    /// Exactly one file (and its module directory) in exactly one package.
    One(FocusedTestTarget),
    /// Several module files tie for the filter, within a package or across
    /// packages; `candidates` are the repo-relative files, sorted.
    Ambiguous {
        filter: String,
        candidates: Vec<String>,
    },
    /// Nothing, or not a cargo test command.
    None,
}

/// What this branch may be widened to: files and directories (no trailing
/// slash), sorted and deduplicated, ownership already applied; and the
/// filters that tied between several module files, as `<filter> (a, b)`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(super) struct Widenable {
    pub files: Vec<String>,
    pub dirs: Vec<String>,
    pub ambiguous: Vec<String>,
}

/// What `prepare_worktree_wave` recorded for the coder and the result: what
/// was widened (files, and directories with a trailing `/`), and the
/// filters that were ambiguous and so widened nothing.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(super) struct FocusedTestTargets {
    pub widened: Vec<String>,
    pub ambiguous: Vec<String>,
}

/// Long options of `cargo test` / `cargo nextest run` that consume the next
/// token when not written `--opt=value`.
const LONG_WITH_VALUE: &[&str] = &[
    "package",
    "exclude",
    "test",
    "bin",
    "example",
    "bench",
    "features",
    "target",
    "target-dir",
    "manifest-path",
    "jobs",
    "build-jobs",
    "profile",
    "cargo-profile",
    "nextest-profile",
    "config",
    "config-file",
    "tool-config-file",
    "color",
    "message-format",
    "filterset",
    "filter-expr",
    "workspace-remap",
    "partition",
    "retries",
    "test-threads",
    "archive-file",
    "extract-to",
    "failure-output",
    "success-output",
    "status-level",
    "final-status-level",
    "run-ignored",
    "timings",
];

/// Short options that consume the next token when not written `-x=value`
/// or `-xvalue`.
const SHORT_WITH_VALUE: &[&str] = &["p", "F", "j", "E", "Z", "P", "J"];

/// The selection `command` makes, read from its tokens.
pub(super) fn selection(command: &str) -> Selection {
    let tokens: Vec<&str> = command.split_whitespace().collect();
    let Some(start) = subcommand_end(&tokens) else {
        return Selection::None;
    };
    let mut integration: Option<String> = None;
    let mut filter: Option<&str> = None;
    let mut index = start;
    while index < tokens.len() {
        let token = tokens[index];
        if matches!(token, "--" | "&&" | "||" | ";" | "|") {
            break;
        }
        if let Some(rest) = token.strip_prefix("--") {
            let (name, inline) = split_inline(rest);
            if name == "test" {
                integration = inline
                    .map(str::to_string)
                    .or_else(|| tokens.get(index + 1).map(|value| value.to_string()));
            }
            index += if inline.is_none() && LONG_WITH_VALUE.contains(&name) {
                2
            } else {
                1
            };
            continue;
        }
        if let Some(rest) = token.strip_prefix('-')
            && !rest.is_empty()
        {
            let (name, inline) = split_inline(rest);
            index += if inline.is_none() && SHORT_WITH_VALUE.contains(&name) {
                2
            } else {
                1
            };
            continue;
        }
        if filter.is_none() {
            filter = Some(token);
        }
        index += 1;
    }
    if let Some(name) = integration {
        let name = name.trim_matches(|c| c == '"' || c == '\'');
        let plain = !name.is_empty()
            && name
                .chars()
                .all(|ch| ch.is_alphanumeric() || ch == '_' || ch == '-');
        return if plain {
            Selection::IntegrationTest(name.to_string())
        } else {
            Selection::None
        };
    }
    let Some(filter) = filter else {
        return Selection::None;
    };
    let filter = filter.trim_matches(|c| c == '"' || c == '\'');
    let segments: Vec<&str> = filter.split("::").collect();
    let valid = segments.iter().all(|segment| {
        !segment.is_empty() && segment.chars().all(|ch| ch.is_alphanumeric() || ch == '_')
    });
    if !valid {
        return Selection::None;
    }
    Selection::Module(segments.into_iter().map(str::to_string).collect())
}

/// The index just past `cargo test` or `cargo nextest run`; `None` for any
/// other command, including `cargo clippy`, `cargo fmt` and `cargo build`.
fn subcommand_end(tokens: &[&str]) -> Option<usize> {
    let cargo = tokens
        .iter()
        .position(|token| *token == "cargo" || token.ends_with("/cargo"))?;
    match tokens.get(cargo + 1).copied() {
        Some("test") => Some(cargo + 2),
        Some("nextest") if tokens.get(cargo + 2).copied() == Some("run") => Some(cargo + 3),
        _ => None,
    }
}

fn split_inline(rest: &str) -> (&str, Option<&str>) {
    match rest.split_once('=') {
        Some((name, value)) => (name, Some(value)),
        None => (rest, None),
    }
}

/// What `command` resolves to under `repo_root`: the one file and module
/// directory exactly one package holds, an ambiguous tie, or nothing.
pub(super) fn resolution(repo_root: &Path, command: &str) -> Resolution {
    let (filter, per_package): (String, Vec<Resolution>) = match selection(command) {
        Selection::Module(segments) => {
            let modules: Vec<&str> = segments.iter().map(String::as_str).collect();
            let per_package = test_baseline_owner::package_source_roots(repo_root, command)
                .iter()
                .map(|src| super::focused_test_resolve::resolve_module(repo_root, src, &modules))
                .collect();
            (segments.join("::"), per_package)
        }
        Selection::IntegrationTest(name) => {
            let per_package = test_baseline_owner::package_dirs_for(repo_root, command)
                .iter()
                .map(|dir| {
                    let tests = if dir.is_empty() {
                        "tests".to_string()
                    } else {
                        format!("{dir}/tests")
                    };
                    let stem = format!("{tests}/{name}");
                    [format!("{stem}.rs"), format!("{stem}/main.rs")]
                        .into_iter()
                        .find(|candidate| repo_root.join(candidate).is_file())
                        .map(|file| Resolution::One(FocusedTestTarget { file, dir: stem }))
                        .unwrap_or(Resolution::None)
                })
                .collect();
            (name, per_package)
        }
        Selection::None => return Resolution::None,
    };
    let mut candidates: Vec<String> = Vec::new();
    let mut ambiguous = false;
    let mut found: Vec<FocusedTestTarget> = Vec::new();
    for resolved in per_package {
        match resolved {
            Resolution::One(target) => {
                candidates.push(target.file.clone());
                found.push(target);
            }
            Resolution::Ambiguous {
                candidates: files, ..
            } => {
                ambiguous = true;
                candidates.extend(files);
            }
            Resolution::None => {}
        }
    }
    candidates.sort();
    candidates.dedup();
    found.dedup();
    match (ambiguous, found.as_slice()) {
        (false, []) => Resolution::None,
        (false, [one]) => Resolution::One(one.clone()),
        _ => Resolution::Ambiguous { filter, candidates },
    }
}

/// `a/b/c.rs` → `a/b/c`; `a/b/c/mod.rs` → `a/b/c`.
pub(super) fn module_dir(file: &str) -> String {
    match file.strip_suffix("/mod.rs") {
        Some(parent) => parent.to_string(),
        None => file.strip_suffix(".rs").unwrap_or(file).to_string(),
    }
}

/// The files and directories the branch's `commands` may be widened to:
/// each resolved file that no OTHER task declares (this task's own and
/// nobody's are kept), and its directory when no other task declares a
/// path within it.
pub(super) fn widenable(
    repo_root: &Path,
    universe: Option<&WorkflowV2TaskUniverse>,
    own_task_ids: &[String],
    own_targets: &[String],
    commands: &[String],
) -> Widenable {
    let mut widened = Widenable::default();
    for command in commands {
        let target = match resolution(repo_root, command) {
            Resolution::One(target) => target,
            Resolution::Ambiguous { filter, candidates } => {
                widened
                    .ambiguous
                    .push(format!("{filter} ({})", candidates.join(", ")));
                continue;
            }
            Resolution::None => continue,
        };
        let owner =
            test_baseline_owner::ownership(universe, own_task_ids, own_targets, &target.file);
        if matches!(owner, Ownership::Other(_)) {
            continue;
        }
        let dir_is_shared = universe.is_some_and(|universe| {
            another_task_declares_within(universe, own_task_ids, &target.dir)
        });
        if !dir_is_shared {
            widened.dirs.push(target.dir);
        }
        widened.files.push(target.file);
    }
    widened.files.sort();
    widened.files.dedup();
    widened.dirs.sort();
    widened.dirs.dedup();
    widened.ambiguous.sort();
    widened.ambiguous.dedup();
    widened
}

/// Whether a task outside `own_task_ids` declares `dir` itself, a path
/// above it, or a path inside it.
fn another_task_declares_within(
    universe: &WorkflowV2TaskUniverse,
    own_task_ids: &[String],
    dir: &str,
) -> bool {
    universe
        .tasks
        .iter()
        .filter(|task| !own_task_ids.contains(&task.canonical_task_id))
        .any(|task| {
            crate::v2::script::declared_writes(universe, &task.canonical_task_id)
                .iter()
                .any(|declared| {
                    crate::v2::write_mode::paths_overlap(declared.trim_end_matches('/'), dir)
                })
        })
}

/// The sentences the coder is told: what was widened (files, and
/// directories with a trailing `/`), and which filters tied between
/// several modules and widened nothing; empty when neither applies.
pub(super) fn preamble(targets: &FocusedTestTargets) -> String {
    let mut text = String::new();
    if !targets.widened.is_empty() {
        text.push_str(&format!(
            "\nFocused-test modules: the files and module directories your declared focused test \
             commands resolve to are added to your declared targets, so you can add or change tests \
             there ({}). A test file another task declares stays that task's and is not listed.\n",
            targets.widened.join(", ")
        ));
    }
    if !targets.ambiguous.is_empty() {
        text.push_str(&format!(
            "\nFocused-test filters that matched several modules and widened nothing — narrow the \
             filter to the module you mean: {}.\n",
            targets.ambiguous.join("; ")
        ));
    }
    text
}

/// Record what was widened (and what was ambiguous) on the branch result,
/// so it is visible next to the scope the gates judged.
pub(super) fn stamp_result(result: &mut crate::WorkflowV2Result, targets: &FocusedTestTargets) {
    if targets.widened.is_empty() && targets.ambiguous.is_empty() {
        return;
    }
    if !result.data.is_object() {
        result.data = serde_json::json!({});
    }
    if !targets.widened.is_empty() {
        result.data["focused_test_targets_widened"] = serde_json::json!(targets.widened);
    }
    if !targets.ambiguous.is_empty() {
        result.data["focused_test_targets_ambiguous"] = serde_json::json!(targets.ambiguous);
    }
}

#[cfg(test)]
#[path = "focused_test_targets_tests.rs"]
mod tests;
