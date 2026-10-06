//! The evidence index a decomposition-time claim is tested against.
//!
//! Built from the repository the task set records (`repository.lock`) at its
//! recorded base commit, plus the paths the task set itself declares it will
//! create or change. Nothing here indexes semantically and nothing writes: it
//! lists the base tree once, reads the blobs a claim names (base commit and
//! checkout, so an author who read either is never contradicted by the other),
//! and reads Cargo manifests once when a declared verifier is a cargo command.

use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

use archon_workflow::repository_record::{
    RepositoryRecordV1, RepositoryTree, UNBORN_BASE_COMMIT, normalize_relative,
};

/// Where a path stands against the index.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Resolved {
    /// At the base commit or in the checkout, under this repository path.
    Exists(String),
    /// Not there yet; these tasks declare they will create it (or a file
    /// under it, for a directory).
    Derived(String, BTreeSet<String>),
    Missing(String),
}

pub(super) struct EvidenceIndex {
    tree: RepositoryTree,
    /// Repository path → the tasks that declare they create or change it.
    declared: BTreeMap<String, BTreeSet<String>>,
    words: RefCell<HashMap<String, HashSet<String>>>,
    packages: RefCell<Option<BTreeMap<String, String>>>,
}

impl EvidenceIndex {
    pub(super) fn load(record: &RepositoryRecordV1) -> archon_workflow::WorkflowResult<Self> {
        Ok(Self {
            tree: RepositoryTree::load(record)?,
            declared: BTreeMap::new(),
            words: RefCell::default(),
            packages: RefCell::default(),
        })
    }

    /// Record that `task` declares it creates or changes `relative`.
    pub(super) fn declare(&mut self, relative: &str, task: &str) {
        self.declared
            .entry(normalize_relative(relative))
            .or_default()
            .insert(task.to_string());
    }

    pub(super) fn root_text(&self) -> String {
        self.tree.root().display().to_string()
    }

    pub(super) fn base(&self) -> &str {
        self.tree.base_commit()
    }

    /// A repository-relative spelling of `token`, `None` when it is an
    /// absolute path outside the recorded repository.
    pub(super) fn relative(&self, token: &str) -> Option<String> {
        self.tree.relative_to_root(token)
    }

    /// Does `relative` point into the repository at all: its first component
    /// is there or declared, or it resolves (possibly as a suffix of a
    /// repository path written relative to a subdirectory).
    pub(super) fn is_repository_path(&self, relative: &str) -> bool {
        let first = relative.split('/').next().unwrap_or_default();
        self.tree.exists_at_base(first)
            || self.tree.root().join(first).exists()
            || self
                .declared
                .keys()
                .any(|path| path.split('/').next() == Some(first))
            || !matches!(self.resolve(relative), Resolved::Missing(_))
    }

    pub(super) fn resolve(&self, relative: &str) -> Resolved {
        let path = normalize_relative(relative);
        if self.present(&path) {
            return Resolved::Exists(path);
        }
        let owners =
            self.owners(|declared| declared == path || declared.starts_with(&format!("{path}/")));
        if !owners.is_empty() {
            return Resolved::Derived(path, owners);
        }
        // Written relative to a package or subdirectory: `tests/x.rs`.
        let suffix = format!("/{path}");
        if let Some(found) = self
            .tree
            .paths_at_base()
            .iter()
            .find(|p| p.ends_with(&suffix))
        {
            return Resolved::Exists(found.clone());
        }
        let owners = self.owners(|declared| declared.ends_with(&suffix));
        if !owners.is_empty() {
            return Resolved::Derived(path, owners);
        }
        Resolved::Missing(path)
    }

    pub(super) fn is_directory(&self, path: &str) -> bool {
        self.tree.is_dir_at_base(path) || self.tree.root().join(path).is_dir()
    }

    fn present(&self, path: &str) -> bool {
        !path.is_empty() && (self.tree.exists_at_base(path) || self.tree.root().join(path).exists())
    }

    fn owners(&self, matches: impl Fn(&str) -> bool) -> BTreeSet<String> {
        self.declared
            .iter()
            .filter(|(path, _)| matches(path))
            .flat_map(|(_, owners)| owners.iter().cloned())
            .collect()
    }

    pub(super) fn declared_by(&self, path: &str) -> BTreeSet<String> {
        self.declared.get(path).cloned().unwrap_or_default()
    }

    /// Does the file at `path` (base commit or checkout) contain `word` as a
    /// whole identifier.
    pub(super) fn contains_word(&self, path: &str, word: &str) -> bool {
        if !self.words.borrow().contains_key(path) {
            let mut words = HashSet::new();
            for text in [self.base_text(path), self.checkout_text(path)] {
                words.extend(identifiers(&text));
            }
            self.words.borrow_mut().insert(path.to_string(), words);
        }
        self.words
            .borrow()
            .get(path)
            .is_some_and(|words| words.contains(word))
    }

    /// Does any file at the base commit contain `word` as a whole word.
    /// One `git grep` per call: asked only when no declared file settles it.
    pub(super) fn in_repository(&self, word: &str) -> bool {
        if self.base() == UNBORN_BASE_COMMIT {
            return false;
        }
        archon_shell::spawn::command("git")
            .arg("-C")
            .arg(self.tree.root())
            .args([
                "grep",
                "-q",
                "-I",
                "-w",
                "-F",
                "-e",
                word,
                self.base(),
                "--",
            ])
            .status()
            .is_ok_and(|status| status.success())
    }

    fn base_text(&self, path: &str) -> String {
        if self.base() == UNBORN_BASE_COMMIT || !self.tree.exists_at_base(path) {
            return String::new();
        }
        archon_shell::spawn::command("git")
            .arg("-C")
            .arg(self.tree.root())
            .args(["cat-file", "blob", &format!("{}:{path}", self.base())])
            .output()
            .ok()
            .filter(|output| output.status.success())
            .map(|output| String::from_utf8_lossy(&output.stdout).into_owned())
            .unwrap_or_default()
    }

    fn checkout_text(&self, path: &str) -> String {
        std::fs::read(self.tree.root().join(path))
            .map(|bytes| String::from_utf8_lossy(&bytes).into_owned())
            .unwrap_or_default()
    }

    /// The directory (`""` for the root) of the Cargo manifest at the base
    /// commit whose `[package]` is `name`.
    pub(super) fn package_dir(&self, name: &str) -> Option<String> {
        self.packages().get(name).cloned()
    }

    /// Every package directory at the base commit.
    pub(super) fn package_dirs(&self) -> Vec<String> {
        self.packages().into_values().collect()
    }

    fn packages(&self) -> BTreeMap<String, String> {
        if self.packages.borrow().is_none() {
            let mut packages = BTreeMap::new();
            for path in self.tree.paths_at_base() {
                let Some(dir) = manifest_dir(path) else {
                    continue;
                };
                if let Some(package) = package_name(&self.base_text(path)) {
                    packages.entry(package).or_insert(dir);
                }
            }
            *self.packages.borrow_mut() = Some(packages);
        }
        self.packages.borrow().clone().unwrap_or_default()
    }

    /// A new package some task declares: a declared `Cargo.toml` not at base
    /// whose directory is named after the package.
    pub(super) fn declared_package(&self, name: &str) -> Option<(String, BTreeSet<String>)> {
        self.declared.iter().find_map(|(path, owners)| {
            let dir = manifest_dir(path)?;
            let named = dir.rsplit('/').next() == Some(name);
            (named && !self.tree.exists_at_base(path)).then(|| (dir, owners.clone()))
        })
    }

    /// Does the manifest in `dir` declare a target entry named `name`.
    pub(super) fn manifest_names(&self, dir: &str, name: &str) -> bool {
        let manifest = join(dir, "Cargo.toml");
        let wanted = format!("\"{name}\"");
        [self.base_text(&manifest), self.checkout_text(&manifest)]
            .iter()
            .any(|text| {
                text.lines().any(|line| {
                    let line = line.trim();
                    line.starts_with("name") && line.contains('=') && line.ends_with(&wanted)
                })
            })
    }
}

pub(super) fn join(dir: &str, path: &str) -> String {
    if dir.is_empty() {
        path.to_string()
    } else {
        format!("{dir}/{path}")
    }
}

fn manifest_dir(path: &str) -> Option<String> {
    if path == "Cargo.toml" {
        return Some(String::new());
    }
    path.strip_suffix("/Cargo.toml").map(str::to_string)
}

fn package_name(manifest: &str) -> Option<String> {
    let mut in_package = false;
    for line in manifest.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            in_package = line == "[package]";
            continue;
        }
        if in_package
            && let Some(value) = line.strip_prefix("name")
            && let Some(value) = value.trim_start().strip_prefix('=')
        {
            return Some(value.trim().trim_matches('"').to_string());
        }
    }
    None
}

fn identifiers(text: &str) -> impl Iterator<Item = String> + '_ {
    text.split(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
        .filter(|word| !word.is_empty())
        .map(str::to_string)
}
