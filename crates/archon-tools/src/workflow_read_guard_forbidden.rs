//! Refusing a file-mutating tool call at a path the task forbids (Issue-30).
//!
//! The task's `Files Forbidden to Change` list reaches the guard the way the
//! focused tests do: the workflow write layer stamps the normalised patterns
//! on the branch input, the host dispatch reads them back and scopes them
//! here as a task-local around the agent call, and the guard built inside
//! that call picks them up at construction. The pipeline constructs the
//! guard per session and cannot be handed the list directly.
//!
//! This is the CHEAP half of the enforcement: a Write, Edit, ApplyPatch,
//! NotebookEdit or LargeEditBegin at a forbidden file costs one refused
//! tool call instead of a rejected branch. A Bash edit (`sed -i`, a heredoc)
//! cannot be seen from here, so the capture-time backstop in the write
//! layer's scope grant is what makes the rule hold; live on wf-719ff3b0
//! `agents-14-1`, four forbidden files were edited through ordinary Edit
//! calls, which this would have refused one by one.
//!
//! The judgement is repo-relative, and an agent names a file from whichever
//! checkout it addresses — its worktree, the canonical repository, or the
//! project artifact root — so every root the host knows is tried, as given
//! and canonicalised (`/var` against `/private/var` on macOS). A relative
//! path is relative to the working directory, which is the worktree root.
//! An absolute path under none of the roots is not judged: it is outside
//! every write root and the path guard refuses it on its own.

use std::path::{Path, PathBuf};

use archon_write_plan::ForbiddenPaths;
use serde_json::Value;

tokio::task_local! { static FORBIDDEN: ForbiddenPathScope; }

/// The forbidden patterns and the roots a tool path is relativised against.
#[derive(Debug, Clone, Default)]
pub struct ForbiddenPathScope {
    paths: ForbiddenPaths,
    roots: Vec<PathBuf>,
}

impl ForbiddenPathScope {
    /// `patterns` are wire patterns from the write layer (or raw entries: the
    /// matcher reads either); `roots` are absolute directories, deduplicated
    /// with their canonical spellings here.
    pub fn new(patterns: &[String], roots: &[String]) -> Self {
        let mut resolved: Vec<PathBuf> = Vec::new();
        for root in roots {
            let given = PathBuf::from(root);
            let canonical = std::fs::canonicalize(&given).unwrap_or_else(|_| given.clone());
            for candidate in [given, canonical] {
                if !resolved.contains(&candidate) {
                    resolved.push(candidate);
                }
            }
        }
        // Longest first, so a worktree nested under the project root strips
        // to the worktree-relative path rather than to `worktrees/x/…`.
        resolved.sort_by_key(|root| std::cmp::Reverse(root.as_os_str().len()));
        Self {
            paths: ForbiddenPaths::from_entries(patterns),
            roots: resolved,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.paths.is_empty()
    }

    /// The refusal for `name` called with `input`, or `None` when the call
    /// does not mutate a file or its target is not forbidden.
    pub(super) fn refusal(&self, name: &str, input: &Value) -> Option<String> {
        if self.is_empty() || !mutates_a_file(name) {
            return None;
        }
        let target = ["file_path", "path"]
            .iter()
            .find_map(|key| input.get(*key).and_then(Value::as_str))?;
        let relative = self.repo_relative(Path::new(target.trim()))?;
        self.paths.matches(&relative).then(|| {
            format!(
                "Error: {target} is forbidden by the task's Files Forbidden to Change list; \
                 leave it unchanged and report the need as a residual gap."
            )
        })
    }

    fn repo_relative(&self, path: &Path) -> Option<String> {
        if path.is_relative() {
            return Some(
                path.to_string_lossy()
                    .replace(std::path::MAIN_SEPARATOR, "/"),
            );
        }
        self.roots
            .iter()
            .find_map(|root| path.strip_prefix(root).ok())
            .map(|relative| relative.to_string_lossy().replace('\\', "/"))
    }
}

/// The tools whose input names the file they will change. The staged
/// large-edit mutations carry only an `edit_id`; the file was named, and is
/// judged, at `LargeEditBegin`.
fn mutates_a_file(name: &str) -> bool {
    matches!(
        name,
        "Write" | "Edit" | "MultiEdit" | "ApplyPatch" | "NotebookEdit" | "LargeEditBegin"
    )
}

/// Scope the forbidden paths for the guard built inside `work`.
pub async fn scope_forbidden_paths<T>(
    scope: ForbiddenPathScope,
    work: impl std::future::Future<Output = T>,
) -> T {
    FORBIDDEN.scope(scope, work).await
}

/// The scope in effect, if any: read once at guard construction.
pub(super) fn current() -> Option<ForbiddenPathScope> {
    FORBIDDEN
        .try_with(Clone::clone)
        .ok()
        .filter(|scope| !scope.is_empty())
}

#[cfg(test)]
#[path = "workflow_read_guard_forbidden_tests.rs"]
mod tests;
