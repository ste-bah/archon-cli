//! Refusing a file-tool write into a checkout the agent was isolated from
//! (Issue-213 C3; see [`crate::spawn_placement`]).
//!
//! Judged on the path the agent NAMED, anchored at its workspace, on what that
//! path RESOLVES to, and on where its links lead when followed one by one
//! ([`follow_links`]). The first catches naming the other checkout outright,
//! which is how the canonical tree was edited from a worktree; the others
//! catch a link inside the workspace that points into it, including a
//! DANGLING one: `canonicalize` cannot resolve a link to a file that does not
//! exist yet, and a write through it creates that file in the sealed tree.
//! Each is judged against the checkout that owns it, so a sibling worktree
//! made after the spawn is sealed like the rest.

use std::path::{Component, Path, PathBuf};

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
    judge(requested, ctx, None)?;
    let followed = follow_links(requested);
    for path in [resolved, followed.as_path()] {
        judge(path, ctx, Some(requested))?;
    }
    Ok(())
}

/// Links in a path are followed at most this many times, as the kernel does.
const MAX_LINK_HOPS: usize = 40;

/// `path` with every symbolic link in it followed, component by component,
/// through `read_link`: a link whose target does not exist is followed all
/// the same, which `canonicalize` cannot do.
pub(crate) fn follow_links(path: &Path) -> PathBuf {
    let mut current = normalise(path);
    for _ in 0..MAX_LINK_HOPS {
        let parts: Vec<Component<'_>> = current.components().collect();
        let mut prefix = PathBuf::new();
        let mut next = None;
        for (index, part) in parts.iter().enumerate() {
            prefix.push(part);
            let is_link =
                std::fs::symlink_metadata(&prefix).is_ok_and(|meta| meta.file_type().is_symlink());
            if let Some(target) = is_link.then(|| std::fs::read_link(&prefix).ok()).flatten() {
                let base = prefix.parent().unwrap_or_else(|| Path::new("/"));
                let rest: PathBuf = parts[index + 1..].iter().collect();
                next = Some(normalise(&base.join(target).join(rest)));
                break;
            }
        }
        match next {
            Some(redirected) => current = redirected,
            None => return current,
        }
    }
    current
}

/// `.` dropped and `..` applied, without touching the filesystem.
fn normalise(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for part in path.components() {
        match part {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            other => out.push(other),
        }
    }
    out
}

/// `named` is the path the agent asked for when `path` is where it leads.
fn judge(path: &Path, ctx: &ToolContext, named: Option<&Path>) -> Result<(), String> {
    let Some(checkout) = sealed_checkout_of(path, ctx) else {
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
    // Named in the workspace, landing in the sealed checkout: a link. Telling
    // the agent to write "the same path in its workspace" would send it
    // straight back through the link.
    if let Some(named) = named.filter(|named| sealed_checkout_of(named, ctx).is_none()) {
        return Err(crate::path_guard::guard_refusal(format!(
            "Path '{}' is a link that leads into {}, a checkout this agent was isolated \
             from, and it cannot be written through. If the host shared it into your \
             workspace, it is shared to read; report what you needed to change there in \
             your envelope.",
            named.display(),
            checkout.display()
        )));
    }
    Err(crate::path_guard::guard_refusal(format!(
        "Path '{}' is in {}, a checkout this agent was isolated from: it may read it but \
         never write it. Make the change in your own workspace ({}) at the same relative \
         path.",
        path.display(),
        checkout.display(),
        ctx.working_dir.display()
    )))
}

fn sealed_checkout_of(path: &Path, ctx: &ToolContext) -> Option<PathBuf> {
    sealed_checkout(path, &ctx.working_dir, &ctx.sealed_repositories)
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
