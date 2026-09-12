//! Staging and committing a large edit, in the execution world.
//!
//! Every byte this module reads or writes goes through [`ToolContext::fs`], the
//! same seam `Write`, `Edit` and `ApplyPatch` use. It did not before: it called
//! `std::fs` directly, so under a sandbox whose world is not the host — and
//! under docker's `/workspace` naming — the staged copy, the metadata and the
//! commit all landed somewhere other than the file the agent was editing, with
//! nothing reporting the split. A tool that writes the host while its shell
//! runs elsewhere is a sandbox that reads as enforced and is not.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::filesystem::FileSystem;
use crate::path_guard::resolve_existing_write_target;
use crate::tool::ToolContext;

const SESSION_DIR: &str = ".archon/large-edits";
const META_FILE: &str = "metadata.json";
const STAGED_FILE: &str = "staged.txt";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct LargeEditMeta {
    pub edit_id: String,
    pub target_path: String,
    pub original_hash: String,
}

#[derive(Debug, Clone)]
pub(super) struct LargeEditSession {
    pub dir: PathBuf,
    pub meta: LargeEditMeta,
}

pub(super) async fn begin(file_path: &str, ctx: &ToolContext) -> Result<LargeEditSession, String> {
    let fs = ctx.fs();
    let target = resolve_existing_write_target(file_path, ctx)?;
    let original = fs
        .read(&target)
        .await
        .map_err(|e| format!("Failed to read target '{}': {e}", target.display()))?;
    let edit_id = Uuid::new_v4().to_string();
    let dir = root_dir(ctx, fs.as_ref()).await?.join(&edit_id);
    fs.create_dir_all(&dir).await.map_err(|e| {
        format!(
            "Failed to create large edit session '{}': {e}",
            dir.display()
        )
    })?;

    fs.write(&staged_path(&dir), &original).await.map_err(|e| {
        format!(
            "Failed to write large edit staged copy '{}': {e}",
            staged_path(&dir).display()
        )
    })?;

    let meta = LargeEditMeta {
        edit_id,
        target_path: target.display().to_string(),
        original_hash: content_hash(&original),
    };
    save_meta(fs.as_ref(), &dir, &meta).await?;
    Ok(LargeEditSession { dir, meta })
}

pub(super) async fn load(edit_id: &str, ctx: &ToolContext) -> Result<LargeEditSession, String> {
    validate_edit_id(edit_id)?;
    let fs = ctx.fs();
    let dir = root_dir(ctx, fs.as_ref()).await?.join(edit_id);
    let raw = fs.read_to_string(&meta_path(&dir)).await.map_err(|e| {
        format!(
            "Failed to read large edit metadata for '{edit_id}' at '{}': {e}",
            meta_path(&dir).display()
        )
    })?;
    let meta: LargeEditMeta = serde_json::from_str(&raw)
        .map_err(|e| format!("Failed to parse large edit metadata for '{edit_id}': {e}"))?;
    Ok(LargeEditSession { dir, meta })
}

pub(super) async fn mutate<F>(edit_id: &str, ctx: &ToolContext, edit: F) -> Result<String, String>
where
    F: FnOnce(&str) -> Result<(String, String), String>,
{
    let session = load(edit_id, ctx).await?;
    let fs = ctx.fs();
    let staged = fs
        .read_to_string(&staged_path(&session.dir))
        .await
        .map_err(|e| {
            format!(
                "Failed to read staged content for '{edit_id}' at '{}': {e}",
                staged_path(&session.dir).display()
            )
        })?;
    let (updated, summary) = edit(&staged)?;
    fs.write(&staged_path(&session.dir), updated.as_bytes())
        .await
        .map_err(|e| {
            format!(
                "Failed to write staged content for '{edit_id}' at '{}': {e}",
                staged_path(&session.dir).display()
            )
        })?;
    Ok(summary)
}

pub(super) async fn commit(
    edit_id: &str,
    ctx: &ToolContext,
    required_fragments: &[String],
) -> Result<String, String> {
    let session = load(edit_id, ctx).await?;
    let fs = ctx.fs();
    let target = resolve_existing_write_target(&session.meta.target_path, ctx)?;
    let current = fs
        .read(&target)
        .await
        .map_err(|e| format!("Failed to read target '{}': {e}", target.display()))?;
    let current_hash = content_hash(&current);
    if current_hash != session.meta.original_hash {
        return Err(format!(
            "Target changed since LargeEditBegin. Expected hash {}, found {}. \
             Abort this session or re-read the file and begin a new large edit.",
            session.meta.original_hash, current_hash
        ));
    }

    let staged = fs.read(&staged_path(&session.dir)).await.map_err(|e| {
        format!(
            "Failed to read staged content for '{edit_id}' at '{}': {e}",
            staged_path(&session.dir).display()
        )
    })?;
    verify_required_fragments(&staged, required_fragments)?;
    replace_target_atomically(fs.as_ref(), &target, &staged, edit_id).await?;
    crate::workflow_read_guard::record_write(ctx, &current, &staged);
    // Best effort, as before: the edit has landed, and a session directory that
    // outlives it is litter rather than a failure worth reporting.
    let _ = discard_session_files(fs.as_ref(), &session.dir).await;
    Ok(format!(
        "Committed large edit {edit_id} to {} ({} bytes).",
        target.display(),
        staged.len()
    ))
}

pub(super) async fn abort(edit_id: &str, ctx: &ToolContext) -> Result<String, String> {
    let session = load(edit_id, ctx).await?;
    discard_session_files(ctx.fs().as_ref(), &session.dir).await?;
    Ok(format!("Aborted large edit {edit_id}."))
}

/// Remove a session's staged files, and then the directory itself.
///
/// The directory used to be left behind — not by choice but by the shape of
/// [`FileSystem`], which offered `remove_file` and nothing for directories, and
/// reaching past it to `std::fs` is the exact bug this module was fixed for. An
/// empty husk is inert, but one per large edit accumulates in a working tree
/// forever, and "inert" stops being the right word at a few thousand of them.
/// The trait now expresses the operation, so this does it in the world that
/// holds the files rather than behind that world's back.
///
/// `Unsupported` is the ONE error tolerated: it is the trait's way of saying
/// this world cannot remove directories, which restores exactly the old
/// behaviour for it and nothing more. Any other failure is reported, because a
/// removal that failed for a reason nobody looked at is how the husks got here.
async fn discard_session_files(fs: &dyn FileSystem, dir: &Path) -> Result<(), String> {
    let entries = fs
        .read_dir(dir)
        .await
        .map_err(|e| format!("Failed to list large edit session '{}': {e}", dir.display()))?;
    for entry in entries {
        fs.remove_file(&entry).await.map_err(|e| {
            format!(
                "Failed to remove large edit session file '{}': {e}",
                entry.display()
            )
        })?;
    }
    match fs.remove_dir(dir).await {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::Unsupported => Ok(()),
        Err(error) => Err(format!(
            "Failed to remove large edit session directory '{}': {error}",
            dir.display()
        )),
    }
}

async fn root_dir(ctx: &ToolContext, fs: &dyn FileSystem) -> Result<PathBuf, String> {
    let working_dir = if ctx.working_dir.as_os_str().is_empty() {
        std::env::current_dir().map_err(|e| format!("Failed to resolve current directory: {e}"))?
    } else {
        ctx.working_dir.clone()
    };
    let root = working_dir.join(SESSION_DIR);
    fs.create_dir_all(&root)
        .await
        .map_err(|e| format!("Failed to create large edit root '{}': {e}", root.display()))?;
    Ok(root)
}

async fn save_meta(fs: &dyn FileSystem, dir: &Path, meta: &LargeEditMeta) -> Result<(), String> {
    let raw = serde_json::to_vec_pretty(meta)
        .map_err(|e| format!("Failed to serialize large edit metadata: {e}"))?;
    fs.write(&meta_path(dir), &raw).await.map_err(|e| {
        format!(
            "Failed to write large edit metadata '{}': {e}",
            meta_path(dir).display()
        )
    })
}

/// Write beside the target, then rename onto it — in the world, not on the host.
///
/// [`FileSystem::rename`] exists precisely so this dance survives the move off
/// `std::fs`, so the commit is still all-or-nothing rather than a truncating
/// write that a crash can leave half-applied.
async fn replace_target_atomically(
    fs: &dyn FileSystem,
    target: &Path,
    content: &[u8],
    edit_id: &str,
) -> Result<(), String> {
    let parent = target
        .parent()
        .ok_or_else(|| format!("Target '{}' has no parent directory", target.display()))?;
    let file_name = target
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| format!("Target '{}' has no valid file name", target.display()))?;
    let tmp = parent.join(format!(".{file_name}.{edit_id}.archon-tmp"));
    fs.write(&tmp, content)
        .await
        .map_err(|e| format!("Failed to write temporary target '{}': {e}", tmp.display()))?;
    if let Err(e) = fs.rename(&tmp, target).await {
        let _ = fs.remove_file(&tmp).await;
        return Err(format!(
            "Failed to atomically replace '{}' with '{}': {e}",
            target.display(),
            tmp.display()
        ));
    }
    Ok(())
}

fn verify_required_fragments(content: &[u8], fragments: &[String]) -> Result<(), String> {
    if fragments.is_empty() {
        return Ok(());
    }
    let text = std::str::from_utf8(content)
        .map_err(|e| format!("Staged content is not valid UTF-8: {e}"))?;
    let missing: Vec<_> = fragments
        .iter()
        .filter(|fragment| !fragment.is_empty() && !text.contains(fragment.as_str()))
        .cloned()
        .collect();
    if missing.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "Large edit commit validation failed; staged content is missing required fragments: {}",
            missing.join(", ")
        ))
    }
}

fn validate_edit_id(edit_id: &str) -> Result<(), String> {
    let ok = !edit_id.is_empty()
        && edit_id
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '-');
    if ok {
        Ok(())
    } else {
        Err("edit_id must contain only ASCII letters, digits, and '-'".into())
    }
}

fn meta_path(dir: &Path) -> PathBuf {
    dir.join(META_FILE)
}

fn staged_path(dir: &Path) -> PathBuf {
    dir.join(STAGED_FILE)
}

fn content_hash(bytes: &[u8]) -> String {
    let mut hash = 0xcbf29ce484222325u64;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("{hash:016x}")
}

#[cfg(test)]
#[path = "session_world_tests.rs"]
mod world_tests;
