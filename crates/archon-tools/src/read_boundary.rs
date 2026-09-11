//! Host-only, task-scoped policy carried into child tool contexts.
use std::path::{Path, Component};
use crate::tool::ToolContext;
tokio::task_local! { static DENIED: Vec<String>; }
pub fn current() -> Vec<String> { DENIED.try_with(Clone::clone).unwrap_or_default() }
pub async fn scope<T>(names: Vec<String>, work: impl std::future::Future<Output=T>) -> T {
    DENIED.scope(names, work).await
}
pub(crate) fn check(path: &Path, ctx: &ToolContext) -> Result<(), String> {
    if ctx.denied_directory_names.is_empty() { return Ok(()); }
    // The workspace itself may live beneath a directory with an excluded name.
    // Exclusions apply below allowed roots, never to their external ancestors.
    let roots = std::iter::once(&ctx.working_dir).chain(ctx.extra_dirs.iter());
    let relative = roots.filter_map(|root| {
        let root = std::fs::canonicalize(root).unwrap_or_else(|_| root.clone());
        path.strip_prefix(root).ok()
    }).max_by_key(|p| p.components().count());
    let relative = relative.unwrap_or(path);
    if relative.components().any(|c| matches!(c, Component::Normal(n) if ctx.denied_directory_names.iter().any(|name| n == std::ffi::OsStr::new(name)))) {
        return Err(format!("Path '{}' is in a host-excluded subtree; read current source outside excluded directories", path.display()));
    }
    Ok(())
}
