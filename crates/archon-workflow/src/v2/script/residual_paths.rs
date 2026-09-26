//! Issue-117: the repository files a finding names, who owns each, and which
//! of them one bounded round may be granted.
//!
//! Agent text supplies CANDIDATES only: a token counts as a named file when,
//! with its location suffix removed (`:12`, `:12-14`, `::symbol`, `#L12`), it
//! is a clean repository path to a file that exists under the root. Who owns
//! it is never read from the text: the task universe's declared entries
//! (`files_expected_to_change`, shared-append targets, deliverable contract
//! paths) decide, exact or by a declared directory above it. "No task owns
//! it" must be PROVEN -- every declared entry readable
//! ([`canonical_declared_paths`]) -- or nothing is concluded. Which tasks a
//! file relates to is read from the task files themselves: the tasks whose
//! own text names the path.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use crate::task_universe::WorkflowV2TaskUniverse;
use crate::v2::verification::path_ownership::{
    DeclaredPathForm, canonical_declared_paths, declared_covers, declared_path_form,
};

/// Paths an expansion never opens, whatever a finding says: engine and
/// workflow configuration, the task set and its PRDs, documentation, run
/// state and version-control internals.
const PROTECTED_PREFIXES: [&str; 8] = [
    "docs/", "prds/", "tasks/", ".archon/", ".claude/", ".git/", ".github/", "config/",
];
const PROTECTED_FILES: [&str; 3] = ["config.toml", "archon.toml", ".gitmodules"];
const PROTECTED_BASENAMES: [&str; 2] = [".mcp.json", ".env"];

/// The exact existing repository files `text` names -- explicitly, by a
/// short form, a glob, a brace set or a directory (`residual_patterns`) --
/// repository-relative, sorted, as the working tree holds them.
pub fn named_files(text: &str, root: &Path) -> Vec<String> {
    named_files_at(text, root, None)
}

/// [`named_files`] as the tree at `commit` held them -- the commit the
/// verifier that recorded the text judged, a fixed fact of its record, so
/// the answer is the same at the slot, at dispatch and at the final gate
/// whatever a later round changed. A commit git cannot read, or none, falls
/// back to the working tree.
pub fn named_files_at(text: &str, root: &Path, commit: Option<&str>) -> Vec<String> {
    let files = super::residual_patterns::tree_files(root, commit);
    super::residual_patterns::resolve_named(text, root, &files)
}

/// Whether `relative` is a regular file inside `root`: never a symbolic
/// link, and never a path that resolves outside the repository.
pub fn is_repo_file(root: &Path, relative: &str) -> bool {
    let path = root.join(relative);
    let regular = std::fs::symlink_metadata(&path).is_ok_and(|meta| meta.file_type().is_file());
    let (Ok(resolved), Ok(base)) = (path.canonicalize(), root.canonicalize()) else {
        return false;
    };
    regular && resolved.starts_with(base)
}

/// `raw` without a location suffix or trailing sentence punctuation; `None`
/// when nothing path-shaped remains.
pub(super) fn strip_location(raw: &str) -> Option<&str> {
    // `file.rs::symbol` names a location in `file.rs`.
    let mut token = raw.split("::").next().unwrap_or(raw);
    token = token.split("#L").next().unwrap_or(token);
    loop {
        let trimmed = token.trim_end_matches(['.', ',', ';', ':', '!', '?']);
        // `path:12`, `path:12-14`, `path:12:4` name lines in `path`.
        let stripped = match trimmed.rsplit_once(':') {
            Some((head, tail))
                if !tail.is_empty() && tail.bytes().all(|b| b.is_ascii_digit() || b == b'-') =>
            {
                head
            }
            _ => trimmed,
        };
        if stripped == token {
            break;
        }
        token = stripped;
    }
    let token = token.trim_start_matches("./");
    (token.contains('/') && !token.contains("://")).then_some(token)
}

/// Every declared entry of one task as a repository-relative path (a
/// declared directory keeps no trailing separator).
fn declared_of(
    task: &crate::task_universe::WorkflowV2TaskUniverseTask,
    root: &Path,
) -> Vec<String> {
    let contracts = task.deliverable_contracts.iter().flat_map(|contract| {
        [
            Some(contract.artifact_path.clone()),
            contract.registry_path.clone(),
            contract.instance_source_path.clone(),
        ]
        .into_iter()
        .flatten()
    });
    task.files_expected_to_change
        .iter()
        .chain(&task.shared_append_target_files)
        .filter_map(|entry| super::declared_path(entry))
        .chain(contracts)
        .filter_map(|entry| match declared_path_form(&entry, root) {
            DeclaredPathForm::Repo(path) => Some(path.trim_end_matches("/**").to_string()),
            _ => None,
        })
        .filter(|path| !path.is_empty())
        .collect()
}

/// Every task that declares `path`: the exact file, or a directory above it.
pub fn owners(universe: &WorkflowV2TaskUniverse, path: &str, root: &Path) -> BTreeSet<String> {
    universe
        .tasks
        .iter()
        .filter(|task| {
            declared_of(task, root)
                .iter()
                .any(|declared| declared_covers(declared, path))
        })
        .map(|task| task.canonical_task_id.clone())
        .collect()
}

/// Whether it is PROVEN that no task declares `path`: every declared entry of
/// the universe reads as a path and none covers it.
pub fn provably_unowned(universe: &WorkflowV2TaskUniverse, path: &str, root: &Path) -> bool {
    canonical_declared_paths(universe, root).is_some_and(|declared| {
        !declared
            .keys()
            .any(|entry| declared_covers(entry.trim_end_matches("/**"), path))
    }) && owners(universe, path, root).is_empty()
}

/// Whether an expansion may never open `path`.
pub fn protected(path: &str) -> bool {
    let path = path.trim_start_matches("./");
    let name = path.rsplit('/').next().unwrap_or(path);
    PROTECTED_PREFIXES
        .iter()
        .any(|prefix| path.starts_with(prefix))
        || PROTECTED_FILES.contains(&path)
        || PROTECTED_BASENAMES.contains(&name)
}

/// Each task's own file text, read once: what "the task's contract names
/// this path" is judged against.
pub struct TaskTexts(BTreeMap<String, String>);

impl TaskTexts {
    pub fn read(universe: &WorkflowV2TaskUniverse, root: &Path) -> Self {
        let texts = universe
            .tasks
            .iter()
            .filter_map(|task| {
                let path = PathBuf::from(&task.source_path);
                let path = if path.is_absolute() {
                    path
                } else {
                    root.join(path)
                };
                let text = std::fs::read_to_string(path).ok()?;
                Some((task.canonical_task_id.clone(), text))
            })
            .collect();
        Self(texts)
    }

    /// The tasks whose own file names `path`, repository-relative or
    /// absolute under `root`.
    pub fn naming(&self, path: &str, root: &Path) -> BTreeSet<String> {
        let absolute = root.join(path).to_string_lossy().replace('\\', "/");
        // Its last two segments (`providers/x_store.rs`): the short form a
        // task body cites a file by.
        let short = path
            .rmatch_indices('/')
            .nth(1)
            .map(|(at, _)| &path[at + 1..])
            .unwrap_or(path);
        self.0
            .iter()
            .filter(|(_, text)| {
                text.contains(path) || text.contains(&absolute) || text.contains(short)
            })
            .map(|(task, _)| task.clone())
            .collect()
    }
}

/// The tasks one unowned file relates to: the tasks whose own text names
/// it, else the tasks of the unit whose verifier recorded the finding.
pub fn related_tasks(unit: &BTreeSet<String>, naming: &BTreeSet<String>) -> BTreeSet<String> {
    if naming.is_empty() {
        unit.clone()
    } else {
        naming.clone()
    }
}

/// `files` a round of `tasks` may be granted: proven unowned, never a
/// protected path, and no longer forbidden once the round's own exact lift
/// ([`residual_forbidden`]) applies -- a directory, basename or glob a task
/// forbids stays forbidden, so a file under one is never granted.
pub fn expandable(
    universe: &WorkflowV2TaskUniverse,
    tasks: &BTreeSet<String>,
    files: &BTreeSet<String>,
    root: &Path,
) -> BTreeSet<String> {
    let candidates: Vec<String> = files
        .iter()
        .filter(|file| !protected(file) && provably_unowned(universe, file, root))
        .cloned()
        .collect();
    let ids: Vec<String> = tasks.iter().cloned().collect();
    let forbidden = residual_forbidden(universe, &ids, &candidates);
    candidates
        .into_iter()
        .filter(|file| !forbidden.matches(file))
        .collect()
}

/// The forbidden list of a residual round of `task_ids` opening `files`: the
/// tasks' own lists as a write item of theirs gets them, less only the
/// patterns wholly inside one of the exact, unprotected `files`.
pub fn residual_forbidden(
    universe: &WorkflowV2TaskUniverse,
    task_ids: &[String],
    files: &[String],
) -> archon_write_plan::ForbiddenPaths {
    let own = || {
        universe
            .tasks
            .iter()
            .filter(|task| task_ids.contains(&task.canonical_task_id))
    };
    let mut forbidden = archon_write_plan::ForbiddenPaths::from_entries(
        own().flat_map(|task| task.files_forbidden_to_change.iter()),
    );
    if own().count() > 1 {
        forbidden = forbidden.without_within(own().flat_map(|task| {
            task.files_expected_to_change
                .iter()
                .chain(&task.shared_append_target_files)
                .filter_map(|entry| super::declared_path(entry))
        }));
    }
    forbidden.without_within(
        files
            .iter()
            .filter(|file| !file.ends_with('/') && !file.contains('*') && !protected(file))
            .cloned(),
    )
}

#[cfg(test)]
#[path = "residual_paths_tests.rs"]
mod tests;
