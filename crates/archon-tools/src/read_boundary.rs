//! Host-only, task-scoped policy carried into child tool contexts.
use crate::tool::ToolContext;
use std::path::{Component, Path};
tokio::task_local! { static DENIED: Vec<String>; }
pub fn current() -> Vec<String> {
    DENIED.try_with(Clone::clone).unwrap_or_default()
}
pub async fn scope<T>(names: Vec<String>, work: impl std::future::Future<Output = T>) -> T {
    DENIED.scope(names, work).await
}
/// [`scope`] for synchronous work, such as a launch-time read probe.
pub fn sync_scope<T>(names: Vec<String>, work: impl FnOnce() -> T) -> T {
    DENIED.sync_scope(names, work)
}
pub(crate) fn check(
    path: &Path,
    ctx: &ToolContext,
) -> Result<(), crate::path_guard_error::GuardError> {
    if ctx.denied_directory_names.is_empty() {
        return Ok(());
    }
    // The workspace itself may live beneath a directory with an excluded name.
    // Exclusions apply below allowed roots, never to their external ancestors.
    // The deepest root that holds the path judges it: a root the host named
    // inside an excluded subtree (a run's author context, Issue 288) admits
    // its own files, and every sibling is still judged by the root above.
    // A root is matched as named and as canonical: the path checked first is
    // only normalized lexically, so a root reached through a symlink (macOS
    // `/var` is `/private/var`) must still own the paths beneath its name.
    let roots = std::iter::once(&ctx.working_dir).chain(ctx.extra_dirs.iter());
    let relative = roots
        .flat_map(|root| {
            let canonical = std::fs::canonicalize(root)
                .map(archon_shell::paths::plain)
                .unwrap_or_else(|_| root.clone());
            [root.clone(), canonical]
        })
        .filter_map(|root| archon_shell::paths::comparable_strip_prefix(path, &root))
        .min_by_key(|p| p.components().count());
    let relative = relative.unwrap_or_else(|| path.to_path_buf());
    if relative.components().any(|component| {
        matches!(component, Component::Normal(name) if ctx.denied_directory_names.iter().any(|denied| is_denied_name(name, denied)))
    }) {
        return Err(crate::path_guard::guard_refusal(format!("Path '{}' is in a host-excluded subtree; read current source outside excluded directories", path.display())));
    }
    Ok(())
}

fn is_denied_name(name: &std::ffi::OsStr, denied: &str) -> bool {
    #[cfg(windows)]
    {
        name.to_string_lossy().eq_ignore_ascii_case(denied)
    }
    #[cfg(not(windows))]
    {
        name == std::ffi::OsStr::new(denied)
    }
}
