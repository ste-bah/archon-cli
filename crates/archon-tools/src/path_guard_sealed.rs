//! Refusing a file-tool write into a checkout the agent was isolated from
//! (Issue-213 C3; see [`crate::spawn_placement`]).
//!
//! Judged twice: on the path the agent NAMED, anchored at its workspace, and
//! on what that path RESOLVES to. The first catches naming the other checkout
//! outright, which is how the canonical tree was edited from a worktree; the
//! second catches a link inside the workspace that points into it. Each is
//! judged against the checkout that owns it, so a sibling worktree made after
//! the spawn is sealed like the rest.

use std::path::Path;

use crate::spawn_placement::{in_workflow, sealed_checkout};
use crate::tool::ToolContext;
use crate::workflow_read_guard::spellings;

/// `Err` when `requested` (lexically normalised, absolute) or `resolved` lies
/// in a checkout of a sealed repository other than the agent's own.
///
/// Three host-named places inside a sealed checkout still pass, because the
/// host decides them and the agent cannot widen them:
///
/// - a path the host declared writable (a project artifact it judges where it
///   is);
/// - the run store, when the project keeps it inside the checkout: what may
///   be written there is the run-store guard's judgement, not this one's. A
///   sibling branch worktree inside the store is a checkout of its own, owns
///   the paths beneath it, and so stays sealed;
/// - a declared write root INSIDE the sealed checkout. A write root that is the
///   checkout itself, or contains it, does not reopen it: that is exactly the
///   "isolated, yet writing the tree it was isolated from" this refuses.
pub(crate) fn ensure_not_sealed(
    requested: &Path,
    resolved: &Path,
    ctx: &ToolContext,
) -> Result<(), String> {
    if ctx.sealed_repositories.is_empty() || !in_workflow(ctx) {
        return Ok(());
    }
    for path in [requested, resolved] {
        judge(path, ctx)?;
    }
    Ok(())
}

fn judge(path: &Path, ctx: &ToolContext) -> Result<(), String> {
    let Some(checkout) = sealed_checkout(path, &ctx.working_dir, &ctx.sealed_repositories) else {
        return Ok(());
    };
    let declared = ctx
        .workflow_read_guard
        .as_ref()
        .is_some_and(|guard| guard.declared_writable(path));
    let in_store = ctx.run_store.as_ref().is_some_and(|store| {
        store
            .store_roots()
            .iter()
            .any(|root| within(path, root) && within(root, &checkout))
    });
    let inner_root = ctx
        .write_roots
        .iter()
        .any(|root| within(path, root) && !within(&checkout, root));
    if declared || in_store || inner_root {
        return Ok(());
    }
    Err(format!(
        "Path '{}' is in {}, a checkout this agent was isolated from: it may read it but \
         never write it. Make the change in your own workspace ({}) at the same relative \
         path.",
        path.display(),
        checkout.display(),
        ctx.working_dir.display()
    ))
}

/// Whether `path` is `root` or lies beneath it, under any spelling of either
/// (`/var` is `/private/var` on macOS).
fn within(path: &Path, root: &Path) -> bool {
    if root.as_os_str().is_empty() {
        return false;
    }
    let roots = spellings(root);
    spellings(path)
        .iter()
        .any(|candidate| roots.iter().any(|root| candidate.starts_with(root)))
}

#[cfg(test)]
#[path = "path_guard_sealed_tests.rs"]
mod tests;
