//! Which repository sources a frozen acceptance check's command runs
//! (PLAN-11), resolved generically from the command text and the tree.
//!
//! - `cargo test` / `cargo nextest run` with `--test NAME`: that integration
//!   target's file (a `[[test]]` path, `tests/NAME.rs` or `tests/NAME/main.rs`)
//!   and every module file it loads (`mod x;`, `#[path]`), recursively. A
//!   target not there yet is pinned ABSENT at `tests/NAME.rs`, with its
//!   `tests/NAME/` directory watched: creating it is a pinned-source change.
//! - a test-name filter with no `--test`: the test functions it selects,
//!   searched in the package's `tests/` files (pinned whole) and `src/` files
//!   (pinned as items, [`crate::check_source_rust`]). A filter nothing defines
//!   yet is WATCHED in the package: a landing that defines it is a
//!   pinned-source change.
//! - no selection at all: the package's whole suite, every `tests/` file and
//!   every test function under `src/`.
//! - `bash|sh|python|node|... <path>`, `./path`, `pytest <path>[::name]`:
//!   that script or test file (a pytest directory: every file under it, and
//!   each `conftest.py` above it). A path not there yet is pinned absent.
//! - `bash -c '<script>'` is resolved recursively; `cd DIR &&` moves the cwd.
//!
//! Anything this cannot follow -- a task runner (`make`, `npm run`, `just`),
//! a nextest filterset, a package no manifest names, a path outside both
//! roots -- is returned in [`Resolution::unresolved`] with the reason, and the
//! pin records it; nothing is silently dropped.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::check_source_rust::normalized;

/// The tree a pinned path is relative to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceRoot {
    Repository,
    Project,
}

/// The two roots a check can reach.
#[derive(Debug, Clone, Copy)]
pub struct Roots<'a> {
    pub repository: &'a Path,
    pub project: &'a Path,
}

impl Roots<'_> {
    pub fn of(&self, root: SourceRoot) -> &Path {
        match root {
            SourceRoot::Repository => self.repository,
            SourceRoot::Project => self.project,
        }
    }

    /// Where `rel`, named from `cwd`, is: under `cwd` when it exists there;
    /// else, for a `cwd` in the project, at the same place in the repository
    /// when it exists there (a combined acceptance view overlays the project
    /// on the repository); else under `cwd`, not created yet.
    pub fn locate(&self, cwd: &Path, rel: &str) -> PathBuf {
        let direct = cwd.join(rel);
        if direct.exists() {
            return direct;
        }
        if let Some((SourceRoot::Project, at)) = self.relative(cwd) {
            let overlaid = self.repository.join(at).join(rel);
            if overlaid.exists() {
                return overlaid;
            }
        }
        direct
    }

    /// The same directory in the repository, for a `cwd` in the project.
    pub fn overlaid(&self, cwd: &Path) -> Option<PathBuf> {
        match self.relative(cwd)? {
            (SourceRoot::Project, at) => Some(self.repository.join(at)),
            _ => None,
        }
    }

    /// `path` (absolute) as (root, relative), the more specific root first;
    /// the repository wins a tie.
    pub fn relative(&self, path: &Path) -> Option<(SourceRoot, String)> {
        let path = PathBuf::from(normalized_abs(path));
        let mut best: Option<(usize, SourceRoot, String)> = None;
        for root in [SourceRoot::Repository, SourceRoot::Project] {
            let base = PathBuf::from(normalized_abs(self.of(root)));
            if let Ok(rel) = path.strip_prefix(&base) {
                let depth = base.components().count();
                if best.as_ref().is_none_or(|(d, _, _)| depth > *d) {
                    best = Some((depth, root, normalized(rel)));
                }
            }
        }
        best.map(|(_, root, rel)| (root, rel))
    }
}

/// `path` absolute and canonical as far as it exists: the deepest existing
/// ancestor is canonicalized (a temporary directory may sit behind a link)
/// and the not-yet-created rest appended.
fn normalized_abs(path: &Path) -> String {
    let path = PathBuf::from(format!("/{}", normalized(path)));
    // A link in a source's own place is the source, never what it points
    // at: only its directory is resolved.
    if std::fs::symlink_metadata(&path).is_ok_and(|meta| meta.file_type().is_symlink())
        && let (Some(parent), Some(name)) = (path.parent(), path.file_name())
    {
        return format!("{}/{}", normalized_abs(parent), name.to_string_lossy());
    }
    let mut existing = path.as_path();
    let mut rest = Vec::new();
    while !existing.exists() {
        match (existing.file_name(), existing.parent()) {
            (Some(name), Some(parent)) => {
                rest.push(name.to_os_string());
                existing = parent;
            }
            _ => break,
        }
    }
    let mut canonical = existing
        .canonicalize()
        .unwrap_or_else(|_| existing.to_path_buf());
    canonical.extend(rest.iter().rev());
    format!("/{}", normalized(&canonical))
}

/// One source the command runs: a whole file, or one test function in it
/// (`item`, keyed `name#n`).
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct Found {
    pub root: SourceRoot,
    pub path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub item: Option<String>,
    pub role: String,
}

/// A source the command will run once it exists: a test function named
/// `test_name` defined anywhere under `dir`, or (no name) any file created
/// under `dir`.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct Watch {
    pub root: SourceRoot,
    pub dir: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub test_name: Option<String>,
    #[serde(default)]
    pub exact: bool,
    /// Directories under `dir` that are other packages: a definition there
    /// is not this package's (a package at the tree root has `dir` empty).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub excluded: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Resolution {
    pub found: Vec<Found>,
    pub watches: Vec<Watch>,
    pub unresolved: Vec<String>,
    /// Whether some of the check's logic is written inline in the command
    /// itself (`test -f`, `jq`, `python3 -c`, a here-document): that part is
    /// frozen with the contract text and has no source to pin.
    pub inline: bool,
}

impl Resolution {
    pub(crate) fn file(
        &mut self,
        roots: &Roots,
        abs: &Path,
        role: &str,
    ) -> Option<(SourceRoot, String)> {
        let Some((root, path)) = roots.relative(abs) else {
            self.unresolved.push(format!(
                "{} ({role}) lies outside the repository and the project",
                abs.display()
            ));
            return None;
        };
        self.found.push(Found {
            root,
            path: path.clone(),
            item: None,
            role: role.to_string(),
        });
        Some((root, path))
    }

    fn finish(mut self) -> Self {
        self.found.sort();
        self.found.dedup();
        self.watches.sort();
        self.watches.dedup();
        self.unresolved.sort();
        self.unresolved.dedup();
        self
    }
}

/// Every source `command`, run in `cwd` (absolute), executes from the
/// repository or the project.
pub fn resolve(command: &str, cwd: &Path, roots: &Roots) -> Resolution {
    let mut out = Resolution::default();
    if shell::has_heredoc(command) {
        out.inline = true;
    }
    resolve_into(command, cwd, roots, &mut out, 0);
    crate::check_source_follow::follow(roots, cwd, &mut out);
    out.finish()
}

/// Tools whose behaviour is fixed and whose arguments, written in the
/// command, are the check's own logic.
const INLINE_TOOLS: &[&str] = &[
    "test",
    "[",
    "[[",
    "true",
    "false",
    ":",
    "echo",
    "printf",
    "cat",
    "grep",
    "egrep",
    "fgrep",
    "rg",
    "jq",
    "yq",
    "find",
    "ls",
    "wc",
    "diff",
    "cmp",
    "head",
    "tail",
    "sort",
    "uniq",
    "cut",
    "tr",
    "sed",
    "awk",
    "mkdir",
    "rm",
    "rmdir",
    "cp",
    "mv",
    "touch",
    "mktemp",
    "stat",
    "du",
    "date",
    "sleep",
    "tee",
    "basename",
    "dirname",
    "realpath",
    "readlink",
    "sha256sum",
    "shasum",
    "md5sum",
    "b2sum",
    "xxd",
    "od",
    "seq",
    "expr",
    "bc",
    "tar",
    "gzip",
    "zcat",
    "unzip",
    "file",
];
/// Shell syntax and builtins that run nothing of their own.
const SHELL_SYNTAX: &[&str] = &[
    "if", "then", "else", "elif", "fi", "for", "do", "done", "while", "until", "case", "esac",
    "in", "{", "}", "!", "trap", "set", "export", "exit", "return", "local", "declare", "readonly",
    "unset", "shift", "wait", "pushd", "popd",
];

fn resolve_into(command: &str, cwd: &Path, roots: &Roots, out: &mut Resolution, depth: usize) {
    let mut cwd = cwd.to_path_buf();
    for segment in segments(&words(command)) {
        let words = strip_wrappers(&segment);
        let Some((program, args)) = words.split_first() else {
            continue;
        };
        let name = program.rsplit('/').next().unwrap_or(program);
        match name {
            "cd" => {
                if let Some(dir) = args.first() {
                    cwd = cwd.join(dir);
                }
            }
            "cargo" => super::check_source_cargo::resolve_cargo(args, &cwd, roots, out),
            "bash" | "sh" | "zsh" | "dash" => match args.iter().position(|a| a == "-c") {
                Some(at) if depth < 4 => {
                    if let Some(script) = args.get(at + 1) {
                        resolve_into(script, &cwd, roots, out, depth + 1);
                    }
                }
                Some(_) => out
                    .unresolved
                    .push("a shell script nested four levels deep".into()),
                None => script_arg(args, &cwd, roots, out, name),
            },
            "python" | "python3" => match args.iter().position(|a| a == "-m") {
                Some(at) if args.get(at + 1).is_some_and(|m| m == "pytest") => {
                    pytest(&args[at + 2..], &cwd, roots, out)
                }
                Some(at) => out.unresolved.push(format!(
                    "`{name} -m {}` runs a module this resolver does not follow",
                    args.get(at + 1).map(String::as_str).unwrap_or_default()
                )),
                None if args.iter().any(|a| a == "-c" || a == "-") || args.is_empty() => {
                    out.inline = true
                }
                None => script_arg(args, &cwd, roots, out, name),
            },
            "pytest" | "py.test" => pytest(args, &cwd, roots, out),
            "node" | "bun" | "ruby" | "perl" | "php" | "tsx" | "ts-node"
                if args
                    .iter()
                    .any(|a| matches!(a.as_str(), "-e" | "-p" | "-r" | "--eval")) =>
            {
                out.inline = true
            }
            "node" | "bun" | "ruby" | "perl" | "php" | "tsx" | "ts-node" => {
                script_arg(args, &cwd, roots, out, name)
            }
            "deno" => {
                let rest = args.strip_prefix(&["run".to_string()]).unwrap_or(args);
                script_arg(rest, &cwd, roots, out, name)
            }
            "make" | "npm" | "yarn" | "pnpm" | "just" | "task" | "rake" | "gradle" | "mvn" => {
                out.unresolved.push(format!(
                    "`{}` runs a task-runner recipe whose sources this resolver does not follow",
                    words.join(" ")
                ))
            }
            "source" | "." => match args.first() {
                Some(file) if file.contains('$') || file.contains('`') => {
                    out.unresolved.push(format!(
                        "the command sources `{file}`, a path built at run time; it is not pinned"
                    ))
                }
                Some(file) => {
                    out.file(roots, &roots.locate(&cwd, file), "sourced script");
                }
                None => out.unresolved.push("a `source` with no file".into()),
            },
            "eval" | "exec" => out.unresolved.push(format!(
                "`{}` builds a command at run time that this resolver cannot follow",
                words.join(" ")
            )),
            _ if INLINE_TOOLS.contains(&name) => out.inline = true,
            _ if SHELL_SYNTAX.contains(&name) => {}
            _ if program.contains('/') => {
                let abs = roots.locate(&cwd, program);
                // A binary the repository builds is its implementation, not a
                // test source.
                if !normalized(&abs).split('/').any(|part| part == "target") {
                    out.file(roots, &abs, "executed script");
                }
            }
            _ => out.unresolved.push(format!(
                "`{name}` is a program this resolver does not follow; what it runs is not pinned"
            )),
        }
    }
}

/// The first non-option argument: the script the interpreter runs.
fn script_arg(args: &[String], cwd: &Path, roots: &Roots, out: &mut Resolution, program: &str) {
    if let Some(script) = args.iter().find(|arg| !arg.starts_with('-')) {
        out.file(
            roots,
            &roots.locate(cwd, script),
            &format!("{program} script"),
        );
    }
}

/// `pytest [opts] path[::name]...`: each named file (and every file under a
/// named directory), with each `conftest.py` from its directory up to `cwd`.
fn pytest(args: &[String], cwd: &Path, roots: &Roots, out: &mut Resolution) {
    let mut named = false;
    let mut skip = false;
    for arg in args {
        if std::mem::take(&mut skip) {
            continue;
        }
        if matches!(arg.as_str(), "-k" | "-m" | "-p" | "-c" | "--rootdir" | "-o") {
            skip = true;
            continue;
        }
        if arg.starts_with('-') {
            continue;
        }
        named = true;
        let path = roots.locate(cwd, arg.split("::").next().unwrap_or(arg));
        if path.is_dir() {
            for file in files_under(&path) {
                out.file(roots, &file, "pytest file");
            }
        } else {
            out.file(roots, &path, "pytest file");
        }
        let mut dir = path.parent().map(Path::to_path_buf);
        while let Some(current) = dir {
            if !current.starts_with(cwd) {
                break;
            }
            let conftest = current.join("conftest.py");
            if conftest.is_file() {
                out.file(roots, &conftest, "pytest conftest");
            }
            dir = current.parent().map(Path::to_path_buf);
        }
    }
    if !named {
        out.unresolved.push(format!(
            "pytest discovers its tests from {}; name the test files for their sources to be pinned",
            cwd.display()
        ));
    }
}

/// Every regular file under `dir`, recursively, hidden entries and caches
/// skipped, sorted.
pub(crate) fn files_under(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(current) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&current) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.starts_with('.')
                || matches!(name.as_str(), "target" | "__pycache__" | "node_modules")
            {
                continue;
            }
            match entry.file_type() {
                Ok(kind) if kind.is_dir() => stack.push(path),
                Ok(kind) if kind.is_file() => out.push(path),
                _ => {}
            }
        }
    }
    out.sort();
    out
}

#[path = "check_source_shell.rs"]
mod shell;
pub(crate) use shell::words;
use shell::{segments, strip_wrappers};

#[cfg(test)]
#[path = "check_source_resolve_tests.rs"]
mod tests;
