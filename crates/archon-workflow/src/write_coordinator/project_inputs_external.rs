//! Issue-226: data roots outside both the project and the repository.
//!
//! A declared data root outside both trees is landed only inside a
//! directory the operator listed in the run's policy
//! (`[workflow.acceptance_execution] external_data_roots`, recorded at
//! launch in `v2/generated-metadata.json` beside the acceptance policy).
//! Task text alone never opens a host path:
//!
//! - each listed directory is canonical (every link resolved); one that does
//!   not resolve, is the filesystem root or the home directory, or overlaps
//!   the project or the repository is dropped, with a logged reason;
//! - a declared path is admitted only when it is absolute, has no `..`
//!   component, and -- every link on it resolved, existing or not -- lies
//!   strictly under a listed directory, with no link left below it;
//! - an empty or absent list admits nothing, as before the key existed.
//!
//! An admitted file lands by the project-input landing's own rules (the
//! baseline, the kept copy, the temp+rename write, the append-only log and
//! undo), keyed by its absolute path; what the run keeps of it lives under
//! [`stored_rel`].

use std::path::{Component, Path, PathBuf};

/// The run policy key that lists the directories external roots may lie in.
pub const EXTERNAL_ROOTS_KEY: &str = "workflow.acceptance_execution.external_data_roots";

/// The run's allowlist of external data directories, canonical.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ExternalRoots {
    allowed: Vec<PathBuf>,
}

/// Where an admitted external file lands: the allowlisted directory it lies
/// under (the tree no link below may be followed in) and the file itself.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Admitted {
    pub tree: PathBuf,
    pub destination: PathBuf,
}

/// `path` with every link resolved, existing or not: its nearest existing
/// ancestor canonical, the rest appended. `None` when nothing of it exists.
pub fn resolved(path: &Path) -> Option<PathBuf> {
    let mut rest = Vec::new();
    let mut cursor = path;
    loop {
        if let Ok(base) = cursor.canonicalize().map(archon_shell::paths::plain) {
            return Some(rest.iter().rev().fold(base, |at, part| at.join(part)));
        }
        rest.push(cursor.file_name()?.to_owned());
        cursor = cursor.parent()?;
    }
}

/// Where the run keeps what it captured or replaced of external file `key`
/// (an absolute path), relative to the landing's own record directory: never
/// a project-relative path, which never starts with `.external`.
pub fn stored_rel(key: &str) -> PathBuf {
    let mut out = PathBuf::from(".external");
    for component in Path::new(key).components() {
        match component {
            Component::Normal(part) => out.push(part),
            Component::Prefix(prefix) => {
                let spelled = prefix.as_os_str().to_string_lossy();
                out.push(spelled.replace(|c: char| !c.is_ascii_alphanumeric(), "_"));
            }
            _ => {}
        }
    }
    out
}

/// Where the record of `key` (project-relative or an absolute external
/// path) is kept under a landing's record directory.
pub fn stored(key: &str) -> PathBuf {
    if Path::new(key).is_absolute() {
        stored_rel(key)
    } else {
        PathBuf::from(key)
    }
}

/// The directories between `tree` and `destination`'s parent that do not
/// exist yet, shallowest first: what writing `destination` creates.
pub fn missing_dirs(tree: &Path, destination: &Path) -> Vec<PathBuf> {
    let mut missing = Vec::new();
    let mut cursor = destination.parent();
    while let Some(dir) = cursor.filter(|dir| dir.starts_with(tree) && *dir != tree) {
        if std::fs::symlink_metadata(dir).is_ok() {
            break;
        }
        missing.push(dir.to_path_buf());
        cursor = dir.parent();
    }
    missing.reverse();
    missing
}

/// Remove directories a landing created (`dirs`, shallowest first), deepest
/// first. One already gone is done; one that holds anything now (another
/// file landed there) stays, with every directory above it. Any other
/// failure is named.
pub fn remove_created(dirs: &[PathBuf]) -> Result<(), String> {
    let mut left = Vec::new();
    for dir in dirs.iter().rev() {
        let holds = std::fs::read_dir(dir).map(|mut entries| entries.next().is_some());
        if holds.unwrap_or(false) {
            break;
        }
        match std::fs::remove_dir(dir) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => left.push(format!("{}: {error}", dir.display())),
        }
    }
    if left.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "could not remove the directories it created: {}",
            left.join("; ")
        ))
    }
}

fn overlaps(a: &Path, b: &Path) -> bool {
    a.starts_with(b) || b.starts_with(a)
}

impl ExternalRoots {
    /// The allowlist as already judged (canonical entries), as a context
    /// carries it.
    pub fn from_allowed(allowed: impl IntoIterator<Item = PathBuf>) -> Self {
        Self {
            allowed: allowed.into_iter().collect(),
        }
    }

    /// The canonical allowlisted directories.
    pub fn allowed(&self) -> &[PathBuf] {
        &self.allowed
    }

    pub fn is_empty(&self) -> bool {
        self.allowed.is_empty()
    }

    /// The allowlist the run at `run_root` recorded at launch; empty when it
    /// recorded none or its policy file cannot be read.
    pub fn recorded(run_root: &Path) -> Self {
        std::fs::read(run_root.join("v2/generated-metadata.json"))
            .ok()
            .and_then(|bytes| serde_json::from_slice::<serde_json::Value>(&bytes).ok())
            .map(|value| Self::from_metadata(&value))
            .unwrap_or_default()
    }

    /// [`Self::recorded`] for the run whose v2 store is `v2_root`, as the
    /// strings a project artifact context carries
    /// (`WorkflowV2ProjectArtifactContext::external_roots`): under them an
    /// absolute declared artifact is one the host lands.
    pub fn listed_for_v2_root(v2_root: &Path) -> Vec<String> {
        let Some(run_root) = v2_root.parent().filter(|_| v2_root.ends_with("v2")) else {
            return Vec::new();
        };
        (Self::recorded(run_root).allowed.iter())
            .map(|root| root.display().to_string())
            .collect()
    }

    /// [`Self::recorded`] from the run's parsed policy file.
    pub fn from_metadata(metadata: &serde_json::Value) -> Self {
        let Some(native) = metadata.pointer("/observer_snapshot/native_execution") else {
            return Self::default();
        };
        let listed: Vec<PathBuf> = (native.get("external_data_roots"))
            .and_then(serde_json::Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(serde_json::Value::as_str)
            .map(PathBuf::from)
            .collect();
        if listed.is_empty() {
            return Self::default();
        }
        let trees: Vec<PathBuf> = ["project", "repository"]
            .iter()
            .filter_map(|key| native.pointer(&format!("/policy/{key}"))?.as_str())
            .filter_map(|tree| {
                Path::new(tree)
                    .canonicalize()
                    .map(archon_shell::paths::plain)
                    .ok()
            })
            .collect();
        let home = std::env::var_os("HOME").and_then(|home| {
            Path::new(&home)
                .canonicalize()
                .map(archon_shell::paths::plain)
                .ok()
        });
        let mut allowed = Vec::new();
        for entry in listed {
            match Self::judge(&entry, &trees, home.as_deref()) {
                Ok(canonical) if !allowed.contains(&canonical) => allowed.push(canonical),
                Ok(_) => {}
                Err(why) => eprintln!(
                    "write-coordination: `{EXTERNAL_ROOTS_KEY}` entry {} is ignored: {why}",
                    entry.display()
                ),
            }
        }
        allowed.sort();
        Self { allowed }
    }

    fn judge(entry: &Path, trees: &[PathBuf], home: Option<&Path>) -> Result<PathBuf, String> {
        if !entry.is_absolute() || entry.components().any(|c| c == Component::ParentDir) {
            return Err("not an absolute, normalized path".into());
        }
        let canonical = entry
            .canonicalize()
            .map(archon_shell::paths::plain)
            .map_err(|error| format!("it does not resolve: {error}"))?;
        if !canonical.is_dir() {
            return Err("not a directory".into());
        }
        if canonical.parent().is_none() || Some(canonical.as_path()) == home {
            return Err("the filesystem root or the home directory is never a data root".into());
        }
        if let Some(tree) = trees.iter().find(|tree| overlaps(&canonical, tree)) {
            return Err(format!(
                "it overlaps {}, which lands by its own rules",
                tree.display()
            ));
        }
        Ok(canonical)
    }

    /// The allowlisted directory `path` (canonical) lies strictly under.
    pub fn tree_of(&self, path: &Path) -> Option<&Path> {
        (self.allowed.iter())
            .find(|tree| path.starts_with(tree) && path != tree.as_path())
            .map(PathBuf::as_path)
    }

    /// Where `declared` lands, or why it may not: see the module doc. Every
    /// refusal names the declared root and the policy key.
    pub fn admit(&self, declared: &Path) -> Result<Admitted, String> {
        let root = declared.parent().unwrap_or(declared).display().to_string();
        if !declared.is_absolute() {
            return Err(format!("`{}` is not an absolute path", declared.display()));
        }
        if declared.components().any(|c| c == Component::ParentDir) {
            return Err(format!(
                "external root `{root}` has a `..` component: a declaration that climbs is never admitted by `{EXTERNAL_ROOTS_KEY}`"
            ));
        }
        if self.allowed.is_empty() {
            return Err(format!(
                "external root `{root}` lies outside both the project and the repository, and the run's policy lists no directory in `{EXTERNAL_ROOTS_KEY}`"
            ));
        }
        let destination =
            resolved(declared).ok_or_else(|| format!("external root `{root}` does not resolve"))?;
        let Some(tree) = self.tree_of(&destination) else {
            return Err(format!(
                "external root `{root}` (every link resolved: {}) is not inside any directory listed in `{EXTERNAL_ROOTS_KEY}`",
                destination.parent().unwrap_or(&destination).display()
            ));
        };
        // A link below the allowlisted directory that does not resolve (or
        // that a resolution left in place) is never written through.
        super::refuse_links(tree, &destination).map_err(|error| {
            format!("external root `{root}` is refused under `{EXTERNAL_ROOTS_KEY}`: {error}")
        })?;
        let below = destination.strip_prefix(tree).unwrap_or(&destination);
        if below.components().any(|c| {
            let part = c.as_os_str().to_string_lossy().to_ascii_lowercase();
            part == ".git"
        }) {
            return Err(format!("external root `{root}` holds a `.git` component"));
        }
        Ok(Admitted {
            tree: tree.to_path_buf(),
            destination,
        })
    }
}

#[cfg(test)]
#[path = "project_inputs_external_tests.rs"]
mod tests;
