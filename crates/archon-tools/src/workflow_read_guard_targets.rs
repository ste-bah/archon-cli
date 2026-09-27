//! Refusing a write outside the branch's declared target set at write time
//! (Issue-64).
//!
//! # The gap this closes
//!
//! A write branch's patch is judged against its declared `target_files`,
//! widened by the host to its baseline obligations; a change anywhere else
//! is dropped (unclaimed, out of scope) or the branch is rejected (a path
//! another item in the wave owns). Nothing said so at WRITE time. Live on
//! wf-0ddadd81 `agents-5`, two of five coders ran a declared lint command,
//! met pre-existing diagnostics in files outside their scope, and spent
//! their whole four-hour sessions editing seventeen files they did not own;
//! the first word either heard was the gate's, four hours later. The
//! tree-wide mutator refusal (Issue-13) already stops the formatter shape
//! of this up front; this stops the one-file-at-a-time shape.
//!
//! # How the set reaches the guard
//!
//! Same route as the forbidden paths (Issue-30): the write layer stamps the
//! widened, repo-relative target set on the branch input
//! (`v2::write::declared_targets::stamp`), the host dispatch reads it back
//! (`agent_dispatch_port::declared_targets`) and scopes it as a task-local
//! around the agent call, and the guard built inside picks it up at
//! construction. The set is the coordinator plan's `target_files` AFTER
//! `widen_to_obligations`, plus its directory scopes — exactly what gates 2
//! and 3 will admit — never the item's raw declaration.
//!
//! # The rule
//!
//! A Write, Edit, MultiEdit, ApplyPatch, NotebookEdit or LargeEditBegin —
//! or a shell write whose target the command spells out
//! (`workflow_read_guard_shell_writes`) — whose path resolves under the
//! branch worktree root and is not a declared target, nor under a declared
//! directory, is refused with the path, the declared list, what the gate
//! would do, and what to do instead. A path outside the worktree root (a
//! project artifact root, notes, evidence, `/tmp`) is not judged: it is not
//! part of the patch. A new file counts as a write. One refusal per call,
//! no state. `workflow.generated.enforce_declared_targets = false` turns it
//! off. Like the mutator rule, this is an efficiency guard, not a sandbox.
//!
//! # Issue-120: the scope the landing keeps, not a stricter one
//!
//! The landing does not drop every undeclared change: the ownership grant
//! (Issue-16) keeps one inside the plan's scope roots (Issue-27) that no
//! other item of the wave claims, and declares it in the manifest. Live on
//! wf-0ddadd81 a residual round's Edit of four such files was refused here
//! as "would be lost"; the coder made the same edits with a shell heredoc
//! this guard cannot see, and all four were granted and landed. When the
//! write layer stamps the roots and the other items' claims
//! ([`DeclaredTargetScope::with_grantable`]), a path the grant would keep
//! is admitted, and a refusal says what the landing would do instead: the
//! agent sees exactly the scope the landing enforces. A forbidden path is
//! never admitted as grantable: this rule refuses it itself, for a shell
//! write or a `..`-spelled path the forbidden rule does not see, since the
//! landing rejects a branch that changes one. Only a DECLARED target
//! overrides a forbidden entry, at both layers. If two items of a wave both
//! change the same unclaimed file, both are granted it and the wave's
//! overlap guard refuses the landing: the preamble says so.

use std::path::{Path, PathBuf};

use serde_json::Value;

use super::shell_writes::write_targets;

tokio::task_local! { static DECLARED_TARGETS: DeclaredTargetScope; }

/// The declared targets and the worktree root a tool path is judged against.
#[derive(Debug, Clone, Default)]
pub struct DeclaredTargetScope {
    /// Repo-relative, `/`-separated. A trailing `/` marks a directory scope;
    /// a bare entry is matched as a file and as a directory, since the
    /// declaration cannot say which it is.
    targets: Vec<String>,
    /// The worktree root as given and canonicalised (`/var` against
    /// `/private/var` on macOS); empty when no root was known.
    roots: Vec<PathBuf>,
    /// What else the landing keeps (Issue-120), when the write layer said.
    grantable: Option<Grantable>,
}

/// The rest of the landing scope: the ownership grant keeps an undeclared
/// change inside the plan's scope roots that no other item of the wave
/// claims (Issue-16/27). Entries in the same form as the targets.
#[derive(Debug, Clone, Default)]
struct Grantable {
    /// Directory roots with a trailing `/`, root-level files bare; empty
    /// means no ceiling.
    scope_roots: Vec<String>,
    /// Every other item's declared files and directory scopes.
    claimed: Vec<String>,
}

fn entry_covers(entry: &str, relative: &str) -> bool {
    let dir = entry.trim_end_matches('/');
    entry == relative
        || dir == relative
        || relative
            .strip_prefix(dir)
            .is_some_and(|rest| rest.starts_with('/'))
}

fn clean(entries: &[String]) -> Vec<String> {
    let mut out: Vec<String> = entries
        .iter()
        .map(|t| t.trim().replace('\\', "/"))
        .map(|t| t.trim_start_matches("./").to_string())
        .filter(|t| !t.is_empty())
        .collect();
    out.sort();
    out.dedup();
    out
}

impl DeclaredTargetScope {
    /// `targets` are repo-relative paths from the write layer; `worktree_root`
    /// is the absolute directory the call runs in. Either empty: inert.
    pub fn new(targets: &[String], worktree_root: Option<&str>) -> Self {
        let mut roots = Vec::new();
        if let Some(root) = worktree_root.map(str::trim).filter(|r| !r.is_empty()) {
            let given = PathBuf::from(root);
            let canonical = std::fs::canonicalize(&given).unwrap_or_else(|_| given.clone());
            for candidate in [given, canonical] {
                if !roots.contains(&candidate) {
                    roots.push(candidate);
                }
            }
        }
        Self {
            targets: clean(targets),
            roots,
            grantable: None,
        }
    }

    /// Issue-120: judge an undeclared path as the landing will. The write
    /// layer stamps the plan's scope roots and the other items' claims; a
    /// path inside the roots that no other item claims is granted to this
    /// branch at landing, so it is admitted here too, and a refusal says
    /// what the landing would actually do.
    #[must_use]
    pub fn with_grantable(mut self, scope_roots: &[String], claimed: &[String]) -> Self {
        self.grantable = Some(Grantable {
            scope_roots: clean(scope_roots),
            claimed: clean(claimed),
        });
        self
    }

    pub fn is_empty(&self) -> bool {
        self.targets.is_empty() || self.roots.is_empty()
    }

    /// The refusal for `name` called with `input`, or `None` when the call
    /// writes nothing, names nothing this can read, writes outside the
    /// worktree, or writes a declared target.
    pub(super) fn refusal(
        &self,
        name: &str,
        input: &Value,
        forbidden: Option<&super::ForbiddenPathScope>,
    ) -> Option<String> {
        if self.is_empty() {
            return None;
        }
        if name == "Bash" {
            let command = input.get("command").and_then(Value::as_str)?;
            return write_targets(command)
                .into_iter()
                .find_map(|write| self.judge(&write.path, Some(&write.head), forbidden));
        }
        if !mutates_a_file(name) {
            return None;
        }
        let target = ["file_path", "path"]
            .iter()
            .find_map(|key| input.get(*key).and_then(Value::as_str))?;
        self.judge(target.trim(), None, forbidden)
    }

    /// Whether a file-mutating tool call names a declared target. False for
    /// a call that names no file, a shell call, or an empty scope.
    pub(super) fn declares_call_target(&self, name: &str, input: &Value) -> bool {
        if self.is_empty() || !mutates_a_file(name) {
            return false;
        }
        ["file_path", "path"]
            .iter()
            .find_map(|key| input.get(*key).and_then(Value::as_str))
            .and_then(|target| self.repo_relative(Path::new(target.trim())))
            .is_some_and(|relative| self.declared(&relative))
    }

    fn judge(
        &self,
        named: &str,
        head: Option<&str>,
        forbidden: Option<&super::ForbiddenPathScope>,
    ) -> Option<String> {
        let relative = self.repo_relative(Path::new(named))?;
        if self.declared(&relative) {
            return None;
        }
        let landing = match &self.grantable {
            None => "undeclared changes are dropped from the patch at the gate, so this edit \
                     would be lost"
                .to_string(),
            Some(grant) => {
                let in_roots = grant.scope_roots.is_empty()
                    || !relative.contains('/')
                    || grant.scope_roots.iter().any(|e| entry_covers(e, &relative));
                let holder = grant
                    .claimed
                    .iter()
                    .any(|e| entry_covers(e, &relative) || entry_covers(&relative, e));
                let forbids = forbidden.is_some_and(|f| f.matches_relative(&relative));
                if in_roots && !holder && !forbids {
                    return None;
                }
                if forbids {
                    "the task forbids it, so the landing would refuse this branch's WHOLE patch"
                        .to_string()
                } else if in_roots {
                    "another item of this wave, or a task this branch does not serve, declares \
                     it, so the landing would refuse this branch's WHOLE patch"
                        .to_string()
                } else {
                    format!(
                        "it is outside this branch's scope roots ({}), so the landing would \
                         discard it",
                        grant.scope_roots.join(", ")
                    )
                }
            }
        };
        Some(self.refusal_text(named, head, &landing))
    }

    fn refusal_text(&self, named: &str, head: Option<&str>, landing: &str) -> String {
        const LISTED: usize = 40;
        let mut listed: Vec<&str> = self
            .targets
            .iter()
            .take(LISTED)
            .map(String::as_str)
            .collect();
        let more = self.targets.len().saturating_sub(LISTED);
        let extra = (more > 0).then(|| format!(" (and {more} more)"));
        if let Some(extra) = &extra {
            listed.push(extra);
        }
        let subject = match head {
            Some(head) => format!("`{head}` writes {named}, which"),
            None => format!("Error: {named}"),
        };
        format!(
            "{subject} is not in this branch's declared target_files; {landing}. Declared targets \
             ({}): {}. If the change is genuinely required to satisfy your own focused checks, \
             do not edit the file: record it in residual_gaps naming the file and the owner \
             task (or \"unowned\"). The operator may disable \
             workflow.generated.enforce_declared_targets.",
            self.targets.len(),
            listed.join(", ")
        )
    }

    /// The path relative to the worktree root, or `None` when it lies
    /// outside it (an absolute path under no root, or a relative path that
    /// climbs out). A relative path is relative to the working directory,
    /// which is the worktree root.
    fn repo_relative(&self, path: &Path) -> Option<String> {
        let text = archon_write_plan::lexical_path::portable(&path.to_string_lossy());
        let under_root = if archon_write_plan::lexical_path::rooted(&text) {
            self.roots.iter().find_map(|root| {
                archon_write_plan::lexical_path::under_root(&text, &root.to_string_lossy())
            })?
        } else {
            text
        };
        normalise(&under_root)
    }

    fn declared(&self, relative: &str) -> bool {
        self.targets
            .iter()
            .any(|target| entry_covers(target, relative))
    }
}

/// Lexical normalisation: `.` dropped, `..` folded; `None` when it climbs
/// above the root.
fn normalise(path: &str) -> Option<String> {
    let mut parts: Vec<&str> = Vec::new();
    for segment in path.split('/') {
        match segment {
            "" | "." => {}
            ".." => {
                parts.pop()?;
            }
            other => parts.push(other),
        }
    }
    Some(parts.join("/"))
}

/// The tools whose input names the file they will change; the staged
/// large-edit mutations carry only an `edit_id`, judged at `LargeEditBegin`.
fn mutates_a_file(name: &str) -> bool {
    matches!(
        name,
        "Write" | "Edit" | "MultiEdit" | "ApplyPatch" | "NotebookEdit" | "LargeEditBegin"
    )
}

/// Scope the declared targets for the guard built inside `work`.
pub async fn scope_declared_targets<T>(
    scope: DeclaredTargetScope,
    work: impl std::future::Future<Output = T>,
) -> T {
    DECLARED_TARGETS.scope(scope, work).await
}

/// The scope in effect, if any: read once at guard construction.
pub(super) fn current() -> Option<DeclaredTargetScope> {
    DECLARED_TARGETS
        .try_with(Clone::clone)
        .ok()
        .filter(|scope| !scope.is_empty())
}

#[cfg(test)]
#[path = "workflow_read_guard_targets_tests.rs"]
mod tests;
