use std::fs;
use std::path::{Component, Path, PathBuf};

use crate::tool::ToolContext;

pub(crate) fn resolve_existing_file_path(
    requested_path: &str,
    ctx: &ToolContext,
) -> Result<PathBuf, String> {
    let anchored = anchor_requested_path(requested_path, ctx)?;
    let normalized = normalize_lexically(&anchored)?;
    let resolved = fs::canonicalize(&normalized).map_err(|e| {
        format!(
            "Failed to resolve file path '{}': {e}",
            normalized.display()
        )
    })?;
    ensure_allowed(&resolved, ctx)?;
    Ok(resolved)
}

pub(crate) fn resolve_write_target_path(
    requested_path: &str,
    ctx: &ToolContext,
) -> Result<PathBuf, String> {
    let anchored = anchor_requested_path(requested_path, ctx)?;
    let normalized = normalize_lexically(&anchored)?;
    let resolved = canonicalize_write_target(&normalized)?;
    ensure_allowed(&resolved, ctx)?;
    Ok(resolved)
}

fn anchor_requested_path(requested_path: &str, ctx: &ToolContext) -> Result<PathBuf, String> {
    let path = Path::new(requested_path);
    if path.is_absolute() {
        return Ok(path.to_path_buf());
    }

    let working_dir = working_dir_root(ctx)?;
    Ok(working_dir.join(path))
}

fn working_dir_root(ctx: &ToolContext) -> Result<PathBuf, String> {
    if ctx.working_dir.as_os_str().is_empty() {
        std::env::current_dir().map_err(|e| format!("Failed to resolve current directory: {e}"))
    } else {
        Ok(ctx.working_dir.clone())
    }
}

fn allowed_roots(ctx: &ToolContext) -> Result<Vec<PathBuf>, String> {
    let working_dir = fs::canonicalize(working_dir_root(ctx)?)
        .map_err(|e| format!("Failed to resolve working_dir: {e}"))?;

    let mut roots = vec![working_dir.clone()];
    for extra_dir in &ctx.extra_dirs {
        let rooted = if extra_dir.is_absolute() {
            extra_dir.clone()
        } else {
            working_dir.join(extra_dir)
        };
        let canonical = fs::canonicalize(&rooted).map_err(|e| {
            format!(
                "Failed to resolve extra allowed directory '{}': {e}",
                rooted.display()
            )
        })?;
        roots.push(canonical);
    }

    Ok(roots)
}

fn ensure_allowed(resolved_path: &Path, ctx: &ToolContext) -> Result<(), String> {
    let roots = allowed_roots(ctx)?;
    if roots
        .iter()
        .any(|root| resolved_path == root || resolved_path.starts_with(root))
    {
        return Ok(());
    }

    let allowed = roots
        .iter()
        .map(|root| root.display().to_string())
        .collect::<Vec<_>>()
        .join(", ");
    Err(format!(
        "Path '{}' is outside allowed directories: {allowed}",
        resolved_path.display()
    ))
}

fn canonicalize_write_target(path: &Path) -> Result<PathBuf, String> {
    if path.exists() {
        return fs::canonicalize(path)
            .map_err(|e| format!("Failed to resolve file path '{}': {e}", path.display()));
    }

    let mut missing_components = Vec::new();
    let mut existing = path;
    while !existing.exists() {
        let file_name = existing.file_name().ok_or_else(|| {
            format!(
                "Cannot write to '{}': no existing parent directory",
                path.display()
            )
        })?;
        missing_components.push(file_name.to_owned());
        existing = existing.parent().ok_or_else(|| {
            format!(
                "Cannot write to '{}': no existing parent directory",
                path.display()
            )
        })?;
    }

    let mut resolved = fs::canonicalize(existing).map_err(|e| {
        format!(
            "Failed to resolve parent directory '{}': {e}",
            existing.display()
        )
    })?;
    for component in missing_components.iter().rev() {
        resolved.push(component);
    }

    Ok(resolved)
}

fn normalize_lexically(path: &Path) -> Result<PathBuf, String> {
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::Prefix(prefix) => normalized.push(prefix.as_os_str()),
            Component::RootDir => normalized.push(Path::new(std::path::MAIN_SEPARATOR_STR)),
            Component::CurDir => {}
            Component::ParentDir => {
                if !normalized.pop() {
                    return Err(format!(
                        "Path '{}' cannot traverse above the filesystem root",
                        path.display()
                    ));
                }
            }
            Component::Normal(part) => normalized.push(part),
        }
    }

    Ok(normalized)
}
