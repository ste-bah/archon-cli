use serde_json::json;

use crate::path_guard::resolve_existing_path;
use crate::tool::{
    PermissionLevel, Tool, ToolCapability, ToolContext, ToolResult, WorkingTreeEffect,
};

pub struct GlobTool;

const MAX_MATCHES: usize = 200;
const MAX_OUTPUT_BYTES: usize = 32 * 1024;

#[async_trait::async_trait]
impl Tool for GlobTool {
    fn name(&self) -> &str {
        "Glob"
    }

    fn capability(&self) -> ToolCapability {
        ToolCapability::FILE_READ
    }

    fn description(&self) -> &str {
        "Fast file pattern matching. Returns up to 200 matching file paths sorted by modification time, with an omitted count; narrow the pattern when truncated."
    }

    fn input_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "pattern": {
                    "type": "string",
                    "description": "Glob pattern to match files (e.g., '**/*.rs')"
                },
                "path": {
                    "type": "string",
                    "description": "Directory to search in (defaults to working directory)"
                }
            },
            "required": ["pattern"]
        })
    }

    async fn execute(&self, input: serde_json::Value, ctx: &ToolContext) -> ToolResult {
        let pattern = match input.get("pattern").and_then(|v| v.as_str()) {
            Some(p) => p,
            None => return ToolResult::error("pattern is required and must be a string"),
        };

        let base_dir = match input.get("path").and_then(|v| v.as_str()) {
            Some(path) => match resolve_existing_path(path, ctx) {
                Ok(path) => path,
                Err(err) => return err.into_tool_result(),
            },
            None => match resolve_existing_path(".", ctx) {
                Ok(path) => path,
                Err(err) => return err.into_tool_result(),
            },
        };

        let fs = ctx.fs();
        // The backend glob cannot apply harness-state pruning or ignore files,
        // so Glob uses the bounded walk for every request.
        let matches = bounded_glob(&base_dir, pattern, ctx).await;
        let matched = match matches {
            Ok(paths) => paths,
            Err(e) => {
                return ToolResult::error(format!("Invalid glob pattern: {e}"));
            }
        };

        let mut files: Vec<(std::path::PathBuf, Option<u128>)> = Vec::new();
        for path in matched {
            let mtime = fs.metadata(&path).await.ok().and_then(|m| m.modified_nanos);
            files.push((path, mtime));
        }

        // Sort by mtime, newest first. A path whose world reports no time
        // sorts last rather than as the epoch, so "unknown" cannot masquerade
        // as "oldest".
        files.sort_by_key(|(path, mtime)| (std::cmp::Reverse(*mtime), path.clone()));

        if files.is_empty() {
            return ToolResult::success("No files matched the pattern.");
        }

        let mut result = String::new();
        let mut shown = 0;
        for (path, _) in files.iter().take(MAX_MATCHES) {
            let path = path.to_string_lossy();
            let separator = usize::from(shown > 0);
            if result.len() + separator + path.len() > MAX_OUTPUT_BYTES {
                break;
            }
            if shown > 0 {
                result.push('\n');
            }
            result.push_str(&path);
            shown += 1;
        }
        let omitted = files.len() - shown;
        if omitted > 0 {
            result.push_str(&format!(
                "\n\n[glob truncated: {shown} matches shown; {omitted} matches omitted; narrow the path or pattern]"
            ));
        }

        ToolResult::success(result)
    }

    fn working_tree_effect(&self) -> WorkingTreeEffect {
        WorkingTreeEffect::None
    }

    fn permission_level(&self, _input: &serde_json::Value) -> PermissionLevel {
        PermissionLevel::Safe
    }
}

// Prune before entering directories, rather than filtering stale paths after
// the backend has already traversed every old checkout.
async fn bounded_glob(
    base: &std::path::Path,
    pattern: &str,
    ctx: &ToolContext,
) -> std::io::Result<Vec<std::path::PathBuf>> {
    bounded_glob_with_state_roots(base, pattern, ctx, harness_state_roots(base)).await
}

#[cfg(test)]
pub(crate) async fn bounded_glob_with_test_state_roots(
    base: &std::path::Path,
    pattern: &str,
    ctx: &ToolContext,
    state_roots: Vec<std::path::PathBuf>,
) -> std::io::Result<Vec<std::path::PathBuf>> {
    bounded_glob_with_state_roots(base, pattern, ctx, state_roots).await
}

async fn bounded_glob_with_state_roots(
    base: &std::path::Path,
    pattern: &str,
    ctx: &ToolContext,
    state_roots: Vec<std::path::PathBuf>,
) -> std::io::Result<Vec<std::path::PathBuf>> {
    let full = base.join(pattern);
    let pattern_path = std::path::Path::new(pattern);
    let targeted_roots = state_roots
        .iter()
        .map(|root| root_targeted_by_pattern(base, pattern_path, root))
        .collect::<Vec<_>>();
    let matcher = glob::Pattern::new(&full.to_string_lossy())
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidInput, e))?;
    let fs = ctx.fs();
    let (global_ignores, _) = ignore::gitignore::GitignoreBuilder::new(base).build_global();
    let inherited_ignores = ancestor_ignores(base, ctx).await;
    let mut pending = vec![(base.to_path_buf(), inherited_ignores)];
    let mut seen = std::collections::BTreeSet::new();
    let mut matches = Vec::new();
    while let Some((dir, inherited_ignores)) = pending.pop() {
        let Ok(canonical) = resolve_existing_path(&dir.to_string_lossy(), ctx) else {
            continue;
        };
        if !seen.insert(canonical) {
            continue;
        }
        if (!ctx.denied_directory_names.is_empty() || ctx.run_store.is_some())
            && seen.len() > 20_000
        {
            return Err(std::io::Error::other(
                "Glob directory limit exceeded; narrow the path",
            ));
        }
        let mut ignores = inherited_ignores;
        for name in [".gitignore", ".ignore"] {
            let ignore_path = dir.join(name);
            if let Some(matcher) = read_ignore_matcher(&ignore_path, &dir, fs.as_ref()).await {
                ignores.push(matcher);
            }
        }
        for (path, known_dir) in fs.read_dir_with_types(&dir).await? {
            // A child's type is useful for deciding whether to descend, but
            // files are only resolved/statted when the pattern can match.
            let is_dir = match known_dir {
                Some(is_dir) => is_dir,
                None => match fs.metadata(&path).await {
                    Ok(meta) => meta.is_dir,
                    Err(_) => continue,
                },
            };
            if is_dir
                && ctx.denied_directory_names.iter().any(|name| {
                    path.file_name()
                        .is_some_and(|n| n == std::ffi::OsStr::new(name))
                })
            {
                continue;
            }
            if is_dir
                && state_roots
                    .iter()
                    .zip(&targeted_roots)
                    .any(|(root, targeted)| !targeted && (path == *root || path.starts_with(root)))
                && !ctx
                    .run_store
                    .as_ref()
                    .is_some_and(|scope| scope.leads_to_owned_tree(&path))
            {
                continue;
            }
            let leads_to_owned_tree = ctx
                .run_store
                .as_ref()
                .is_some_and(|scope| scope.leads_to_owned_tree(&path));
            let leads_to_targeted_root =
                state_roots
                    .iter()
                    .zip(&targeted_roots)
                    .any(|(root, targeted)| {
                        *targeted && (root.starts_with(&path) || path.starts_with(root))
                    });
            if !leads_to_owned_tree
                && !leads_to_targeted_root
                && ignores_match(&global_ignores, &ignores, &path, is_dir)
            {
                continue;
            }
            let run_store = ctx.run_store.as_ref();
            let matches_path = matcher.matches_path(&path);
            if matches_path {
                if let Ok(canonical) = resolve_existing_path(&path.to_string_lossy(), ctx) {
                    if !run_store.is_some_and(|store| store.holds_host_records(&canonical)) {
                        matches.push(path.clone());
                    }
                }
            }
            if is_dir {
                // The host's run bookkeeping. Entered anyway while the run's
                // artifact area or the branch worktree lies beneath it —
                // both sit inside the store — and pruned once nothing the
                // agent owns is left below.
                if run_store.is_some_and(|store| store.prunes_walk(&path)) {
                    continue;
                }
                if resolve_existing_path(&path.to_string_lossy(), ctx).is_err() {
                    continue;
                }
                pending.push((path, ignores.clone()));
            }
        }
    }
    Ok(matches)
}

async fn ancestor_ignores(
    base: &std::path::Path,
    ctx: &ToolContext,
) -> Vec<ignore::gitignore::Gitignore> {
    let fs = ctx.fs();
    let mut ancestors = Vec::new();
    let mut current = Some(base);
    let mut git_boundary = None;
    while let Some(dir) = current {
        if fs.metadata(&dir.join(".git")).await.is_ok() {
            git_boundary = Some(dir.to_path_buf());
            break;
        }
        if dir != base {
            ancestors.push(dir.to_path_buf());
        }
        current = dir.parent();
    }

    let repository_boundary = git_boundary.clone();
    let selected = if let Some(boundary) = git_boundary {
        ancestors.push(boundary);
        ancestors.reverse();
        ancestors
    } else if base.starts_with(&ctx.working_dir) {
        let mut ancestors = ancestors
            .into_iter()
            .filter(|dir| dir.starts_with(&ctx.working_dir))
            .collect::<Vec<_>>();
        ancestors.reverse();
        ancestors
    } else {
        Vec::new()
    };

    let mut ignores = Vec::new();
    if let Some(boundary) = repository_boundary {
        if let Some(exclude_path) = repository_exclude_path(&boundary, fs.as_ref()).await {
            if let Some(matcher) = read_ignore_matcher(&exclude_path, &boundary, fs.as_ref()).await
            {
                ignores.push(matcher);
            }
        }
    }
    for dir in selected {
        for name in [".gitignore", ".ignore"] {
            let ignore_path = dir.join(name);
            if let Some(matcher) = read_ignore_matcher(&ignore_path, &dir, fs.as_ref()).await {
                ignores.push(matcher);
            }
        }
    }
    ignores
}

async fn read_ignore_matcher(
    path: &std::path::Path,
    root: &std::path::Path,
    fs: &dyn crate::filesystem::FileSystem,
) -> Option<ignore::gitignore::Gitignore> {
    let bytes = fs.read(path).await.ok()?;
    let mut builder = ignore::gitignore::GitignoreBuilder::new(root);
    for line in String::from_utf8_lossy(&bytes).lines() {
        let _ = builder.add_line(Some(path.to_path_buf()), line);
    }
    builder.build().ok()
}

async fn repository_exclude_path(
    root: &std::path::Path,
    fs: &dyn crate::filesystem::FileSystem,
) -> Option<std::path::PathBuf> {
    let git_path = root.join(".git");
    let git_metadata = fs.metadata(&git_path).await.ok()?;
    let git_dir = if git_metadata.is_dir {
        git_path
    } else {
        let contents = fs.read(&git_path).await.ok()?;
        let text = String::from_utf8_lossy(&contents);
        let line = text.lines().find(|line| line.starts_with("gitdir: "))?;
        let path = std::path::PathBuf::from(line.trim_start_matches("gitdir: "));
        if path.is_absolute() {
            path
        } else {
            root.join(path)
        }
    };
    let common_dir_file = git_dir.join("commondir");
    let common_dir = if fs.metadata(&common_dir_file).await.is_ok() {
        let contents = fs.read(&common_dir_file).await.ok()?;
        let path = std::path::PathBuf::from(String::from_utf8_lossy(&contents).trim());
        if path.is_absolute() {
            path
        } else {
            git_dir.join(path)
        }
    } else {
        git_dir
    };
    Some(common_dir.join("info/exclude"))
}

fn root_targeted_by_pattern(
    base: &std::path::Path,
    pattern: &std::path::Path,
    root: &std::path::Path,
) -> bool {
    use std::path::Component;

    let root_components = match root.strip_prefix(base) {
        Ok(relative) => relative
            .components()
            .filter_map(|component| match component {
                Component::Normal(value) => Some(value.to_os_string()),
                _ => None,
            })
            .collect::<Vec<_>>(),
        Err(_) => {
            if !pattern.is_absolute() {
                return false;
            }
            let mut literal_prefix = std::path::PathBuf::new();
            for component in pattern.components() {
                match component {
                    Component::RootDir | Component::Prefix(_) => {
                        literal_prefix.push(component.as_os_str());
                    }
                    Component::Normal(value) if is_literal_component(value) => {
                        literal_prefix.push(value);
                    }
                    Component::Normal(_) => break,
                    _ => {}
                }
            }
            return literal_prefix.starts_with(root);
        }
    };
    if root_components.is_empty() {
        return false;
    }
    let pattern_components = pattern
        .components()
        .filter_map(|component| match component {
            Component::Normal(value) if is_literal_component(value) => {
                Some(Some(value.to_os_string()))
            }
            Component::Normal(_) => Some(None),
            _ => None,
        })
        .collect::<Vec<_>>();
    pattern_components
        .windows(root_components.len())
        .any(|window| {
            window
                .iter()
                .zip(&root_components)
                .all(|(pattern, root)| pattern.as_ref() == Some(root))
        })
}

fn is_literal_component(component: &std::ffi::OsStr) -> bool {
    let text = component.to_string_lossy();
    !text.contains('*') && !text.contains('?') && !text.contains('[') && !text.contains('{')
}

/// Storage roots from the same path definitions used by their owners. These
/// can be nested under a project when the operator configures cache storage
/// there, so pruning only hidden directory names is not enough.
fn harness_state_roots(base: &std::path::Path) -> Vec<std::path::PathBuf> {
    let mut roots = Vec::new();
    if let Some(root) = crate::agent_records::sessions_root() {
        roots.push(root);
    }
    // WorkflowStore's project-local path, as used by workflow host setup.
    roots.push(base.join(".archon/workflows"));
    roots.push(crate::worktree_manager::WorktreeManager::worktrees_dir());
    roots.push(crate::worktree_manager::WorktreeManager::build_cache_root());
    roots.push(crate::cache_paths::cargo_fallback_root());
    roots
}

fn ignores_match(
    global: &ignore::gitignore::Gitignore,
    local: &[ignore::gitignore::Gitignore],
    path: &std::path::Path,
    is_dir: bool,
) -> bool {
    for matcher in local.iter().rev() {
        match matcher.matched(path, is_dir) {
            ignore::Match::Ignore(_) => return true,
            ignore::Match::Whitelist(_) => return false,
            ignore::Match::None => {}
        }
    }
    matches!(global.matched(path, is_dir), ignore::Match::Ignore(_))
}
