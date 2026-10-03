//! What a spawn's `isolation` field asked for, and which directories its child
//! gets as a result (#236).
//!
//! The field is parsed once, here, with the shared parser in
//! `archon_tools::isolation`. A value that parser does not know fails the
//! spawn and names the value and the field it came from. Before, an unknown
//! value fell through to "nothing asked for", and that is how
//! `workspace-boundary` was ignored on every spawn that sent it.

use std::path::{Path, PathBuf};

use archon_tools::isolation::Isolation;
use archon_tools::subagent_request::SubagentRequest;
use archon_tools::tool::ToolContext;

use crate::agents::CustomAgentDefinition;
use crate::agents::transcript::{InheritedConfinement, RecordedIsolation, SpawnConfinement};

/// The isolation this spawn asked for: the request's own field, else its
/// agent definition's. `Err` carries the refusal text for an unknown value.
pub(super) fn requested(
    request: &SubagentRequest,
    definition: Option<&CustomAgentDefinition>,
) -> Result<Option<Isolation>, String> {
    if let Some(raw) = request.isolation.as_deref() {
        return Isolation::parse(raw, "the spawn request's isolation field")
            .map(Some)
            .map_err(|error| error.to_string());
    }
    let Some(definition) = definition else {
        return Ok(None);
    };
    let Some(raw) = definition.isolation.as_deref() else {
        return Ok(None);
    };
    let source = format!(
        "the isolation field of agent definition '{}'",
        definition.agent_type
    );
    Isolation::parse(raw, &source)
        .map(Some)
        .map_err(|error| error.to_string())
}

/// A child confined to the directory it was given.
///
/// It inherits none of its parent's directories. It may read its working
/// directory, the read roots its caller named and its write roots. It may
/// write only its working directory and its declared write roots.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct WorkspaceBoundary {
    read_roots: Vec<PathBuf>,
}

impl WorkspaceBoundary {
    /// Refuses a relative read root: relative to what is the question a
    /// caller must answer, and guessing could widen the boundary.
    pub(super) fn new(read_roots: &[String]) -> Result<Self, String> {
        let read_roots = read_roots.iter().map(PathBuf::from).collect::<Vec<_>>();
        if let Some(relative) = read_roots.iter().find(|root| !root.is_absolute()) {
            return Err(format!(
                "workspace-boundary read root '{}' is not an absolute path",
                relative.display()
            ));
        }
        Ok(Self { read_roots })
    }

    /// The directories the child may write: its declared write roots and
    /// always its working directory.
    ///
    /// The list is never empty, and that is the point. An empty list means
    /// "unconfined" to the path guard, so every directory the child can read
    /// would also be writable, read roots included.
    pub(super) fn write_roots(&self, declared: Vec<PathBuf>, working_dir: &Path) -> Vec<PathBuf> {
        let mut roots = declared;
        if !roots.iter().any(|root| root == working_dir) {
            roots.push(working_dir.to_path_buf());
        }
        roots
    }

    /// The directories the child may read beyond its working directory:
    /// the named read roots and the write roots, and nothing inherited.
    ///
    /// A root that does not exist is left out with a warning. The path
    /// guard resolves every root on every check, and one root that does
    /// not resolve fails every read. Leaving it out only narrows access.
    pub(super) fn extra_dirs(&self, write_roots: &[PathBuf], working_dir: &Path) -> Vec<PathBuf> {
        let mut dirs: Vec<PathBuf> = Vec::new();
        for root in self.read_roots.iter().chain(write_roots) {
            if root == working_dir || dirs.contains(root) {
                continue;
            }
            if !root.exists() {
                tracing::warn!(
                    root = %root.display(),
                    "workspace-boundary root does not exist; it is not readable"
                );
                continue;
            }
            dirs.push(root.clone());
        }
        dirs
    }
}

/// The confinement a resume of this spawn must restore (#241).
///
/// `child_dir` is the directory the agent started in, before any worktree, so
/// a resume starts there and a worktree rung reuses the checkout made from it.
/// `parent` is the context it was spawned from, which may have confined it
/// further. A resume builds this again from its own run and refuses when the
/// two differ.
pub(super) fn spawn_confinement(
    request: &SubagentRequest,
    prepared: &super::run_prepare::PreparedSubagentRun,
    child_dir: &Path,
    parent: &ToolContext,
) -> SpawnConfinement {
    SpawnConfinement {
        isolation: RecordedIsolation::of(prepared.requested_isolation),
        tier: prepared.tier,
        cwd: child_dir.display().to_string(),
        read_roots: request.read_roots.clone(),
        write_roots: request.write_roots.clone(),
        allowed_tools: request.allowed_tools.clone(),
        model: request.model.clone(),
        max_turns: request.max_turns,
        timeout_secs: request.timeout_secs,
        inherited: InheritedConfinement::of(parent),
    }
}

/// The parent's directories a child without a boundary inherits: the parent's
/// working directory and its extra directories, apart from the child's own.
pub(super) fn inherited_extra_dirs(
    parent_ctx: &ToolContext,
    child_working_dir: &Path,
) -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if !parent_ctx.working_dir.as_os_str().is_empty()
        && parent_ctx.working_dir.as_path() != child_working_dir
    {
        dirs.push(parent_ctx.working_dir.clone());
    }
    for extra_dir in &parent_ctx.extra_dirs {
        let resolved = if extra_dir.is_absolute() {
            extra_dir.clone()
        } else {
            parent_ctx.working_dir.join(extra_dir)
        };
        if !dirs.contains(&resolved) && resolved.as_path() != child_working_dir {
            dirs.push(resolved);
        }
    }
    dirs
}

#[cfg(test)]
#[path = "run_isolation_tests.rs"]
mod tests;
