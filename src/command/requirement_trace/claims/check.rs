//! Testing one observable against the evidence index.
//!
//! Every verdict names what was looked for and where. A refutation is issued
//! only when both the base commit and the checkout lack the thing AND no task
//! in the set declares it will create it: a file a later task writes is not
//! missing, it is derived, and the claim that relies on it stands.

use std::collections::BTreeSet;

use super::index::{EvidenceIndex, Resolved, join};
use super::observe::CommandSubject;

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub(super) enum Verdict {
    Exists(String),
    Derived(String),
    Refuted(String),
    Untested(String),
}

impl Verdict {
    pub(super) fn tested(&self) -> bool {
        matches!(self, Self::Exists(_) | Self::Derived(_))
    }
}

/// The task a claim belongs to, as the checks need it.
pub(super) struct TaskScope<'a> {
    pub(super) task_id: &'a str,
    /// Repository paths this task declares it may create or change.
    pub(super) changeable: &'a BTreeSet<String>,
}

fn owners_text(owners: &BTreeSet<String>) -> String {
    owners.iter().cloned().collect::<Vec<_>>().join(", ")
}

fn short(index: &EvidenceIndex) -> String {
    index.base().chars().take(12).collect()
}

/// A repository path the claim relies on.
pub(super) fn path(index: &EvidenceIndex, relative: &str) -> Verdict {
    match index.resolve(relative) {
        Resolved::Exists(found) => Verdict::Exists(format!("`{found}` is in the repository")),
        Resolved::Derived(found, owners) => Verdict::Derived(format!(
            "`{found}` is not in the repository yet; {} declares it",
            owners_text(&owners)
        )),
        Resolved::Missing(found) => Verdict::Refuted(format!(
            "`{found}` is not in the repository at base commit {} or in the checkout, and no task in the set declares it, so nothing will ever make it exist",
            short(index)
        )),
    }
}

/// A symbol the claim names, in `file` when the body places it there.
pub(super) fn symbol(
    index: &EvidenceIndex,
    scope: &TaskScope<'_>,
    name: &str,
    file: Option<&str>,
) -> Verdict {
    let Some(file) = file else {
        if let Some(found) = scope
            .changeable
            .iter()
            .find(|p| index.contains_word(p, name))
        {
            return Verdict::Exists(format!("`{name}` appears in `{found}`"));
        }
        if scope.changeable.is_empty() {
            if index.in_repository(name) {
                return Verdict::Exists(format!("`{name}` appears in the repository at base"));
            }
            return Verdict::Refuted(format!(
                "`{name}` appears nowhere in the repository at base commit {}, and {} declares no file it may change, so nothing can introduce it",
                short(index),
                scope.task_id
            ));
        }
        return Verdict::Derived(format!(
            "`{name}` is not in {}'s declared files yet; they are where it will be introduced",
            scope.task_id
        ));
    };
    match index.resolve(file) {
        Resolved::Exists(found) if index.is_directory(&found) => Verdict::Untested(format!(
            "`{name}` is placed in directory `{found}`; the index reads files, not directories"
        )),
        Resolved::Exists(found) => {
            if index.contains_word(&found, name) {
                return Verdict::Exists(format!("`{name}` appears in `{found}`"));
            }
            if scope.changeable.contains(&found) {
                return Verdict::Derived(format!(
                    "`{name}` is not in `{found}` yet; {} declares that file",
                    scope.task_id
                ));
            }
            let owners = index.declared_by(&found);
            if !owners.is_empty() {
                return Verdict::Derived(format!(
                    "`{name}` is not in `{found}` yet; {} declares that file",
                    owners_text(&owners)
                ));
            }
            // A line may name a symbol beside a file it does not live in;
            // the claim is false only if the symbol is nowhere at all.
            if index.in_repository(name) {
                return Verdict::Exists(format!(
                    "`{name}` is not in `{found}` but appears elsewhere in the repository at base"
                ));
            }
            Verdict::Refuted(format!(
                "`{name}` appears nowhere in the repository at base commit {}, and the body places it in `{found}`, which no task in the set may change, so `{name}` cannot come to be there",
                short(index)
            ))
        }
        Resolved::Derived(found, owners) => Verdict::Derived(format!(
            "`{name}` will be written in `{found}`, which {} declares",
            owners_text(&owners)
        )),
        Resolved::Missing(found) => Verdict::Untested(format!(
            "`{name}` is placed in `{found}`, which is itself missing; the path's own verdict decides the claim"
        )),
    }
}

/// One thing a declared verifier command asks the repository to have.
pub(super) fn command_subject(
    index: &EvidenceIndex,
    command: &str,
    subject: &CommandSubject,
) -> Verdict {
    match subject {
        CommandSubject::Path(raw) => match index.relative(raw) {
            Some(relative) if index.is_repository_path(&relative) => path(index, &relative),
            Some(relative) => Verdict::Untested(format!(
                "`{command}` names `{relative}`, which is not a repository path, so the index cannot see it"
            )),
            None => outside(raw),
        },
        CommandSubject::Package(name) => {
            if let Some(dir) = index.package_dir(name) {
                return Verdict::Exists(format!(
                    "package `{name}` is `{}`",
                    join(&dir, "Cargo.toml")
                ));
            }
            if let Some((dir, owners)) = index.declared_package(name) {
                return Verdict::Derived(format!(
                    "package `{name}` will be `{}`, which {} declares",
                    join(&dir, "Cargo.toml"),
                    owners_text(&owners)
                ));
            }
            Verdict::Refuted(format!(
                "`{command}` names package `{name}`, but no Cargo.toml at base commit {} is that package and no task in the set declares one, so the command cannot run",
                short(index)
            ))
        }
        CommandSubject::Target(package, kind, name) => {
            target(index, command, package.as_deref(), kind, name)
        }
    }
}

fn target(
    index: &EvidenceIndex,
    command: &str,
    package: Option<&str>,
    kind: &str,
    name: &str,
) -> Verdict {
    let dirs: Vec<String> = match package {
        Some(package) => match index
            .package_dir(package)
            .or_else(|| index.declared_package(package).map(|(dir, _)| dir))
        {
            Some(dir) => vec![dir],
            None => {
                return Verdict::Untested(format!(
                    "{kind} target `{name}` belongs to package `{package}`, whose own verdict decides the command"
                ));
            }
        },
        None => {
            let mut dirs = index.package_dirs();
            dirs.push(String::new());
            dirs
        }
    };
    let mut expected = Vec::new();
    for dir in &dirs {
        let candidates = target_candidates(dir, kind, name, package == Some(name));
        if index.manifest_names(dir, name) {
            return Verdict::Exists(format!(
                "{kind} target `{name}` is declared in `{}`",
                join(dir, "Cargo.toml")
            ));
        }
        for candidate in &candidates {
            match index.resolve(candidate) {
                Resolved::Exists(found) if &found == candidate => {
                    return Verdict::Exists(format!("{kind} target `{name}` is `{found}`"));
                }
                Resolved::Derived(found, owners) if &found == candidate => {
                    return Verdict::Derived(format!(
                        "{kind} target `{name}` will be `{found}`, which {} declares",
                        owners_text(&owners)
                    ));
                }
                _ => {}
            }
        }
        expected.extend(candidates.into_iter().take(1));
    }
    Verdict::Refuted(format!(
        "`{command}` runs {kind} target `{name}`, which no package has at base commit {} (expected {}) and no task in the set declares, so the command cannot run",
        short(index),
        expected
            .iter()
            .map(|p| format!("`{p}`"))
            .collect::<Vec<_>>()
            .join(" or ")
    ))
}

fn target_candidates(dir: &str, kind: &str, name: &str, package_named: bool) -> Vec<String> {
    let folder = match kind {
        "test" => "tests",
        "example" => "examples",
        "bench" => "benches",
        _ => "src/bin",
    };
    let mut candidates = vec![
        join(dir, &format!("{folder}/{name}.rs")),
        join(dir, &format!("{folder}/{name}/main.rs")),
    ];
    if kind == "bin" && package_named {
        candidates.push(join(dir, "src/main.rs"));
    }
    candidates
}

pub(super) fn outside(raw: &str) -> Verdict {
    Verdict::Untested(format!(
        "`{raw}` is outside the recorded repository; the decomposition-time index covers only the repository"
    ))
}
