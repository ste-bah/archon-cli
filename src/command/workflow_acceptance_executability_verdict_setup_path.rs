//! Issue 420 (gap 2): a check whose run stopped at its own setup -- the
//! directory it enters (`cd X`, `pushd X`), the manifest it builds
//! (`--manifest-path X`), the directory a tool works in (`git -C X`) --
//! because that path lies outside the tree it ran in and is absent there
//! gave no verdict: it fails the same way whatever the criterion says, so
//! it proves nothing about the tree.
//!
//! The rule applies only where the host made the tree a check ran in: a
//! copy ([`SiteTree`]: the scratch site, the probe's own hermetic copy).
//! Outside that tree means an absolute path (the copy is made under a fresh
//! directory no check text can name), or a relative path that, walked from
//! the check's working directory and following the `cd`s it made before,
//! climbs above the copy's top or enters a top-level name the copy does not
//! hold (a sibling root of the live layout, such as `../code-root`). On the
//! live tree a path above the working directory may be a real part of the
//! live layout (a sibling repository), so there the rule never applies. A
//! path inside the tree that is absent is the deliverable not there yet: a
//! verdict, as for a program started by a path in the tree (rule 1). A path
//! that expands (`$X`, `~`) is not decided here, and neither is one the
//! output does not report missing (the run then went past it), nor any
//! relative path after a `cd` this rule cannot follow (`cd -`, `cd $X`).

use std::path::{Component, Path};

use super::super::verdict_shell::{Simple, expands};

/// Builtins that enter the directory they are given.
const ENTERS: &[&str] = &["cd", "pushd"];

/// Options whose value is a path the program's setup needs, with the
/// programs that read them so (empty: any program).
const PATH_OPTIONS: &[(&str, &[&str])] = &[
    ("--manifest-path", &[]),
    ("-C", &["git", "make", "gmake", "ninja", "cargo"]),
];

/// How a program reports a path it was given as absent.
const MISSING: &[&str] = &[
    "does not exist",
    "no such file or directory",
    "can't cd to",
    "cannot find",
    "could not find",
    "not found",
];

/// The copy a check ran in: its working directory lies `depth` levels below
/// the copy's top, and the top holds only `roots` (the repository and
/// project copies, and the site's own directories).
#[derive(Clone, Debug)]
pub(crate) struct SiteTree {
    depth: usize,
    roots: Vec<String>,
}

impl SiteTree {
    pub(crate) fn new(depth: usize, roots: &[&str]) -> Self {
        let roots = roots.iter().map(|root| (*root).to_string()).collect();
        Self { depth, roots }
    }
}

/// Why the failed run (non-zero exit, `output`) of `commands` in the copy
/// `tree` gave no verdict under this rule; never on the live tree (`None`).
pub(super) fn refusal(
    code: i32,
    commands: &[Simple],
    output: &str,
    tree: Option<&SiteTree>,
) -> Option<String> {
    let tree = tree.filter(|_| code != 0)?;
    // How deep below the copy's top the working directory is; `None` once
    // a `cd` this rule cannot follow made it unknown.
    let mut depth = Some(tree.depth);
    for command in commands {
        let name = base(&command.word);
        let enters = command.program.is_none() && ENTERS.contains(&name);
        let entered = enters
            .then(|| command.args.iter().find(|arg| !arg.starts_with('-')))
            .flatten();
        let mut named: Vec<(&str, &str)> = entered
            .map(|dir| (name, dir.as_str()))
            .into_iter()
            .collect();
        named.extend(path_options(command, name));
        for (how, path) in named {
            if !expands(path) && outside(path, depth, tree) && reported_missing(output, path) {
                return Some(format!(
                    "it names the path `{path}` ({how}), which lies outside the tree it ran in and is absent there, as its output says: a setup failure of its site, not a failure of its criterion"
                ));
            }
        }
        if enters {
            depth = match entered {
                Some(dir) if !expands(dir) => depth.and_then(|depth| walk(dir, depth, tree)),
                _ => None,
            };
        }
    }
    None
}

/// Whether a failed check stopped at a missing setup path outside its copy.
pub(crate) fn failure(
    command: &str,
    result: &super::CheckResult,
    at: &super::Context,
) -> Option<String> {
    if result.operational_error.is_some() || super::super::baseline::passed(result) {
        return None;
    }
    refusal(
        result.exit_code?,
        &super::simple_commands(command),
        &super::output(result),
        at.tree.as_ref(),
    )
}

/// The program's file name, as a check may start it by a path.
fn base(word: &str) -> &str {
    word.rsplit(['/', '\\']).next().unwrap_or(word)
}

/// The values of every [`PATH_OPTIONS`] option `command` passes.
fn path_options<'a>(command: &'a Simple, name: &'a str) -> Vec<(&'a str, &'a str)> {
    let mut found = Vec::new();
    for (index, arg) in command.args.iter().enumerate() {
        for (option, programs) in PATH_OPTIONS {
            if !programs.is_empty() && (command.program.is_none() || !programs.contains(&name)) {
                continue;
            }
            if arg == option {
                if let Some(value) = command.args.get(index + 1) {
                    found.push((*option, value.as_str()));
                }
            } else if let Some(value) = arg
                .strip_prefix(option)
                .and_then(|rest| rest.strip_prefix('='))
            {
                found.push((*option, value));
            }
        }
    }
    found
}

/// The depth below the copy's top that `path`, walked from `depth`, ends
/// at; `None` when it is absolute, climbs above the top, or enters a
/// top-level name the copy does not hold.
fn walk(path: &str, depth: usize, tree: &SiteTree) -> Option<usize> {
    let mut at = depth;
    for component in Path::new(path).components() {
        match component {
            Component::Prefix(_) | Component::RootDir => return None,
            Component::CurDir => {}
            Component::ParentDir => at = at.checked_sub(1)?,
            Component::Normal(name) => {
                if at == 0 && !tree.roots.iter().any(|root| name == root.as_str()) {
                    return None;
                }
                at += 1;
            }
        }
    }
    Some(at)
}

/// Whether `path` lies outside the copy from a working directory `depth`
/// below its top: always when absolute, never decided for a relative path
/// once the depth is unknown.
fn outside(path: &str, depth: Option<usize>, tree: &SiteTree) -> bool {
    if Path::new(path).has_root() || Path::new(path).is_absolute() {
        return true;
    }
    depth.is_some_and(|depth| walk(path, depth, tree).is_none())
}

/// Whether a line of `output` names `path` and reports it absent.
fn reported_missing(output: &str, path: &str) -> bool {
    let path = path.trim_end_matches('/');
    !path.is_empty()
        && output.lines().any(|line| {
            let lower = line.to_ascii_lowercase();
            line.contains(path) && MISSING.iter().any(|wording| lower.contains(wording))
        })
}

#[cfg(test)]
#[path = "workflow_acceptance_executability_verdict_setup_path_tests.rs"]
mod tests;
