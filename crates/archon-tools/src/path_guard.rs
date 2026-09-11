use std::fs;
use std::path::{Component, Path, PathBuf};

use crate::filesystem::HostWriteTarget;
use crate::tool::ToolContext;

pub(crate) fn resolve_existing_file_path(
    requested_path: &str,
    ctx: &ToolContext,
) -> Result<PathBuf, String> {
    resolve_existing_path(requested_path, ctx)
}

/// An existing file this context is allowed to MODIFY.
///
/// `resolve_existing_file_path` answers "may this be read", and `Edit` used it
/// to decide whether a file may be rewritten. Those are the same question only
/// while writing is unconfined; for an agent sealed into its own workspace they
/// differ, and conflating them is what let a worktree-isolated agent edit the
/// checkout it was branched from.
pub(crate) fn resolve_existing_write_target(
    requested_path: &str,
    ctx: &ToolContext,
) -> Result<PathBuf, String> {
    if let Some(world) = world_path(requested_path, ctx) {
        let admitted = world?;
        ensure_world_write_allowed(&admitted, ctx)?;
        return Ok(admitted);
    }
    let (normalized, resolved) = resolve_existing_host_path(requested_path, ctx)?;
    ensure_write_allowed(&normalized, &resolved, ctx)?;
    Ok(resolved)
}

pub(crate) fn resolve_existing_path(
    requested_path: &str,
    ctx: &ToolContext,
) -> Result<PathBuf, String> {
    if let Some(world) = world_path(requested_path, ctx) {
        return world;
    }
    Ok(resolve_existing_host_path(requested_path, ctx)?.1)
}

/// The normalised and the canonical spelling of an existing host path, both
/// returned because the write guard needs both.
///
/// Containment is decided on the canonical path, since that is the file that
/// will change. Symbolic links have to be judged on the normalised path,
/// because canonicalisation is precisely the step that erases the evidence:
/// once a link has been followed, the result no longer records that it was.
fn resolve_existing_host_path(
    requested_path: &str,
    ctx: &ToolContext,
) -> Result<(PathBuf, PathBuf), String> {
    let anchored = anchor_requested_path(requested_path, ctx)?;
    let normalized = normalize_lexically(&anchored)?;
    crate::read_boundary::check(&normalized, ctx)?;
    let resolved = fs::canonicalize(&normalized).map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            format!("File does not exist: {}", normalized.display())
        } else {
            format!(
                "Failed to resolve file path '{}': {e}",
                normalized.display()
            )
        }
    })?;
    ensure_allowed(&resolved, ctx)?;
    Ok((normalized, resolved))
}

pub(crate) fn resolve_write_target_path(
    requested_path: &str,
    ctx: &ToolContext,
) -> Result<PathBuf, String> {
    if let Some(world) = world_path(requested_path, ctx) {
        let admitted = world?;
        ensure_world_write_allowed(&admitted, ctx)?;
        return Ok(admitted);
    }
    let anchored = anchor_requested_path(requested_path, ctx)?;
    let normalized = normalize_lexically(&anchored)?;
    crate::read_boundary::check(&normalized, ctx)?;
    let resolved = canonicalize_write_target(&normalized)?;
    ensure_allowed(&resolved, ctx)?;
    ensure_write_allowed(&normalized, &resolved, ctx)?;
    Ok(resolved)
}

/// Apply write confinement to a path the execution world named itself.
///
/// [`world_path`] exists because a container path cannot be canonicalised on
/// the host, and it returns before the host guard for that reason. That early
/// return was also skipping the write-root check, so an agent refused
/// `{working_dir}/src/lib.rs` could write the identical file by calling it
/// `/workspace/src/lib.rs`. Confinement that a spelling change defeats is not
/// confinement, and it is exactly the "present but inert" shape this guard
/// exists to avoid.
///
/// The world is asked where the write lands on this machine rather than being
/// trusted to bound itself, because the two boundaries are different: the mount
/// keeps a path inside the *workspace*, and confinement keeps it inside the
/// *declared roots*, which are usually narrower and sometimes elsewhere
/// entirely.
fn ensure_world_write_allowed(world_target: &Path, ctx: &ToolContext) -> Result<(), String> {
    if ctx.write_roots.is_empty() {
        return Ok(());
    }
    match ctx.fs().host_write_target(world_target) {
        HostWriteTarget::Ephemeral => Ok(()),
        HostWriteTarget::Host(host) => {
            let resolved = canonicalize_write_target(&host)?;
            ensure_write_allowed(&host, &resolved, ctx)
        }
        HostWriteTarget::Unknown => Err(format!(
            "Cannot write '{}': this agent's writes are confined to directories on this \
             machine, and the execution world cannot say which file on this machine that \
             path names. Confinement is expressed in host paths and cannot be evaluated \
             against a world that does not share the host filesystem.",
            world_target.display()
        )),
    }
}

/// Refuse a write outside the directories this context may write to.
///
/// Applied on top of [`ensure_allowed`], never instead of it: a path must still
/// be somewhere the context can see before the question of writing it arises.
///
/// `ToolContext::write_roots` empty means writing is unconfined and this is a
/// no-op, which is what every interactive session has and keeps: a directory
/// added with `/add-dir` is one the user asked for and intends to edit in, so
/// `extra_dirs` is not silently demoted to read-only.
///
/// It is populated for an agent given its own workspace. Such an agent still
/// receives the checkout it was branched from in `extra_dirs`, because reading
/// it is legitimate and usually necessary — but one list served reads and
/// writes alike, so being allowed to read the real checkout meant being allowed
/// to write it. A worktree-isolated write agent did exactly that: it edited
/// five files in the canonical tree, including 111 lines of debug scaffolding,
/// while the run recorded its repository root as a worktree. Nothing refused
/// the write and nothing noticed; a person reading `git status` found it hours
/// later.
///
/// `requested_path` is the normalised spelling the caller asked for and
/// `resolved_path` what it canonicalises to. Both are taken because
/// containment and symbolic links are decided on different ones — see
/// [`crate::path_guard_symlink`], which owns the second question.
fn ensure_write_allowed(
    requested_path: &Path,
    resolved_path: &Path,
    ctx: &ToolContext,
) -> Result<(), String> {
    if ctx.write_roots.is_empty() {
        return Ok(());
    }
    // Before containment, deliberately: a link out of the roots fails both, and
    // "that name is a link" is the diagnosis the agent can act on.
    crate::path_guard_symlink::reject_symlinked_target(requested_path)?;
    let mut roots = Vec::new();
    for root in &ctx.write_roots {
        // A root that cannot be resolved is not silently skipped: dropping it
        // would quietly widen the confinement it exists to impose.
        let canonical = fs::canonicalize(root)
            .map_err(|e| format!("Failed to resolve write root '{}': {e}", root.display()))?;
        roots.push(canonical);
    }
    if let Some(root) = roots
        .iter()
        .find(|root| resolved_path == *root || resolved_path.starts_with(root))
    {
        return crate::path_guard_symlink::reject_symlinked_descent(
            root,
            requested_path,
            resolved_path,
        );
    }
    let allowed = roots
        .iter()
        .map(|root| root.display().to_string())
        .collect::<Vec<_>>()
        .join(", ");
    Err(format!(
        "Path '{}' is readable but outside this agent's writable directories: {allowed}. \
         Make the change in your own workspace; another agent owns that tree.",
        resolved_path.display()
    ))
}

/// The execution world's answer for a path it names itself, if it has one.
///
/// Under a sandbox the model's paths come from two places: the ones `Read` and
/// `Glob` handed it, which are host paths, and the ones `Bash` printed, which
/// are the world's — `/workspace/src/main.rs` under docker, the remote workdir
/// over ssh. The host guard is right about the first and cannot be right about
/// the second: canonicalising a container path on the host fails, so the tool
/// refused a file that plainly existed. That was the whole gap between the
/// filesystem seam and the tools that use it (#201).
///
/// The world vouches for its own paths because it already bounds them — a
/// container path that would climb out of the mount is refused by the
/// translation itself. Host paths still go through the host guard below, so
/// nothing here widens what a tool may touch on this machine.
fn world_path(requested_path: &str, ctx: &ToolContext) -> Option<Result<PathBuf, String>> {
    let fs = ctx.fs.as_ref()?;
    let admitted = fs.admit_world_path(Path::new(requested_path))?;
    Some(
        admitted.map_err(|error| format!("Failed to resolve file path '{requested_path}': {error}"))
            .and_then(|path| {
                crate::read_boundary::check(&path, ctx)?;
                if !ctx.denied_directory_names.is_empty() {
                    match fs.host_write_target(&path) {
                        HostWriteTarget::Host(host) => {
                            crate::read_boundary::check(&canonicalize_write_target(&host)?, ctx)?;
                        }
                        _ => return Err("Excluded-subtree policy requires a host-resolvable filesystem".into()),
                    }
                }
                Ok(path)
            }),
    )
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
    crate::read_boundary::check(resolved_path, ctx)?;
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

#[cfg(test)]
#[path = "path_guard_write_roots_tests.rs"]
mod write_roots_tests;

#[cfg(test)]
#[path = "path_guard_world_write_tests.rs"]
mod world_write_tests;
