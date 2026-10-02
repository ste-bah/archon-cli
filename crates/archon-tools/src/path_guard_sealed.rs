//! Refusing a file-tool write into a checkout the agent was isolated from
//! (Issue-213 C3; see [`crate::spawn_placement`]).
//!
//! Decided on the path the agent NAMED, anchored at its workspace, never on
//! what links resolve to: the host shares a few canonical directories into a
//! worktree by symlink on purpose (dependency trees, ignored project state),
//! and a write beneath one of those is the sharing working as designed. What
//! is refused is naming the other checkout itself, which is how the canonical
//! tree was edited from a worktree.

use std::path::{Path, PathBuf};

use crate::tool::ToolContext;
use crate::workflow_read_guard::spellings;

/// `Err` when `requested` (lexically normalised, absolute) lies in one of the
/// context's sealed checkouts and not in its own workspace.
///
/// Three host-named places inside a sealed checkout still pass, because the
/// host decides them and the agent cannot widen them:
///
/// - a path the host declared writable (a project artifact it judges where it
///   is);
/// - the run store, when the project keeps it inside the checkout: what may
///   be written there is the run-store guard's judgement, not this one's. A
///   sibling branch worktree inside the store is a sealed checkout of its own,
///   and the most specific seal decides, so it stays sealed;
/// - a declared write root INSIDE the sealed checkout. A write root that is the
///   checkout itself, or contains it, does not reopen it: that is exactly the
///   "isolated, yet writing the tree it was isolated from" this refuses.
pub(crate) fn ensure_not_sealed(requested: &Path, ctx: &ToolContext) -> Result<(), String> {
    if ctx.sealed_roots.is_empty() || within(requested, &ctx.working_dir) {
        return Ok(());
    }
    // The most specific seal decides: a branch worktree inside the run store
    // inside the canonical checkout is judged as that worktree.
    let Some(sealed) = ctx
        .sealed_roots
        .iter()
        .filter(|root| within(requested, root))
        .max_by_key(|root| root.components().count())
    else {
        return Ok(());
    };
    let declared = ctx
        .workflow_read_guard
        .as_ref()
        .is_some_and(|guard| guard.declared_writable(requested));
    let in_store = ctx.run_store.as_ref().is_some_and(|store| {
        store
            .store_roots()
            .iter()
            .any(|root| within(requested, root) && within(root, sealed))
    });
    let inner_root = ctx
        .write_roots
        .iter()
        .any(|root| within(requested, root) && !within(sealed, root));
    if declared || in_store || inner_root {
        return Ok(());
    }
    Err(format!(
        "Path '{}' is in {}, a checkout this agent was isolated from: it may read it but \
         never write it. Make the change in your own workspace ({}) at the same relative \
         path.",
        requested.display(),
        sealed.display(),
        ctx.working_dir.display()
    ))
}

/// Whether `path` is `root` or lies beneath it, under any spelling of either
/// (`/var` is `/private/var` on macOS).
fn within(path: &Path, root: &Path) -> bool {
    if root.as_os_str().is_empty() {
        return false;
    }
    let roots: Vec<PathBuf> = spellings(root);
    spellings(path)
        .iter()
        .any(|candidate| roots.iter().any(|root| candidate.starts_with(root)))
}

#[cfg(test)]
#[path = "path_guard_sealed_tests.rs"]
mod tests;
