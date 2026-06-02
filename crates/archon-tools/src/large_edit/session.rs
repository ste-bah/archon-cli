use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::path_guard::resolve_existing_file_path;
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

pub(super) fn begin(file_path: &str, ctx: &ToolContext) -> Result<LargeEditSession, String> {
    let target = resolve_existing_file_path(file_path, ctx)?;
    let original = fs::read(&target)
        .map_err(|e| format!("Failed to read target '{}': {e}", target.display()))?;
    let edit_id = Uuid::new_v4().to_string();
    let dir = root_dir(ctx)?.join(&edit_id);
    fs::create_dir_all(&dir).map_err(|e| {
        format!(
            "Failed to create large edit session '{}': {e}",
            dir.display()
        )
    })?;

    fs::write(staged_path(&dir), &original).map_err(|e| {
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
    save_meta(&dir, &meta)?;
    Ok(LargeEditSession { dir, meta })
}

pub(super) fn load(edit_id: &str, ctx: &ToolContext) -> Result<LargeEditSession, String> {
    validate_edit_id(edit_id)?;
    let dir = root_dir(ctx)?.join(edit_id);
    let raw = fs::read_to_string(meta_path(&dir)).map_err(|e| {
        format!(
            "Failed to read large edit metadata for '{edit_id}' at '{}': {e}",
            meta_path(&dir).display()
        )
    })?;
    let meta: LargeEditMeta = serde_json::from_str(&raw)
        .map_err(|e| format!("Failed to parse large edit metadata for '{edit_id}': {e}"))?;
    Ok(LargeEditSession { dir, meta })
}

pub(super) fn mutate<F>(edit_id: &str, ctx: &ToolContext, edit: F) -> Result<String, String>
where
    F: FnOnce(&str) -> Result<(String, String), String>,
{
    let session = load(edit_id, ctx)?;
    let staged = fs::read_to_string(staged_path(&session.dir)).map_err(|e| {
        format!(
            "Failed to read staged content for '{edit_id}' at '{}': {e}",
            staged_path(&session.dir).display()
        )
    })?;
    let (updated, summary) = edit(&staged)?;
    fs::write(staged_path(&session.dir), updated).map_err(|e| {
        format!(
            "Failed to write staged content for '{edit_id}' at '{}': {e}",
            staged_path(&session.dir).display()
        )
    })?;
    Ok(summary)
}

pub(super) fn commit(
    edit_id: &str,
    ctx: &ToolContext,
    required_fragments: &[String],
) -> Result<String, String> {
    let session = load(edit_id, ctx)?;
    let target = resolve_existing_file_path(&session.meta.target_path, ctx)?;
    let current = fs::read(&target)
        .map_err(|e| format!("Failed to read target '{}': {e}", target.display()))?;
    let current_hash = content_hash(&current);
    if current_hash != session.meta.original_hash {
        return Err(format!(
            "Target changed since LargeEditBegin. Expected hash {}, found {}. \
             Abort this session or re-read the file and begin a new large edit.",
            session.meta.original_hash, current_hash
        ));
    }

    let staged = fs::read(staged_path(&session.dir)).map_err(|e| {
        format!(
            "Failed to read staged content for '{edit_id}' at '{}': {e}",
            staged_path(&session.dir).display()
        )
    })?;
    verify_required_fragments(&staged, required_fragments)?;
    replace_target_atomically(&target, &staged, edit_id)?;
    let _ = fs::remove_dir_all(&session.dir);
    Ok(format!(
        "Committed large edit {edit_id} to {} ({} bytes).",
        target.display(),
        staged.len()
    ))
}

pub(super) fn abort(edit_id: &str, ctx: &ToolContext) -> Result<String, String> {
    let session = load(edit_id, ctx)?;
    fs::remove_dir_all(&session.dir).map_err(|e| {
        format!(
            "Failed to remove large edit session '{}': {e}",
            session.dir.display()
        )
    })?;
    Ok(format!("Aborted large edit {edit_id}."))
}

fn root_dir(ctx: &ToolContext) -> Result<PathBuf, String> {
    let working_dir = if ctx.working_dir.as_os_str().is_empty() {
        std::env::current_dir().map_err(|e| format!("Failed to resolve current directory: {e}"))?
    } else {
        ctx.working_dir.clone()
    };
    let root = working_dir.join(SESSION_DIR);
    fs::create_dir_all(&root)
        .map_err(|e| format!("Failed to create large edit root '{}': {e}", root.display()))?;
    Ok(root)
}

fn save_meta(dir: &Path, meta: &LargeEditMeta) -> Result<(), String> {
    let raw = serde_json::to_vec_pretty(meta)
        .map_err(|e| format!("Failed to serialize large edit metadata: {e}"))?;
    fs::write(meta_path(dir), raw).map_err(|e| {
        format!(
            "Failed to write large edit metadata '{}': {e}",
            meta_path(dir).display()
        )
    })
}

fn replace_target_atomically(target: &Path, content: &[u8], edit_id: &str) -> Result<(), String> {
    let parent = target
        .parent()
        .ok_or_else(|| format!("Target '{}' has no parent directory", target.display()))?;
    let file_name = target
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| format!("Target '{}' has no valid file name", target.display()))?;
    let tmp = parent.join(format!(".{file_name}.{edit_id}.archon-tmp"));
    fs::write(&tmp, content)
        .map_err(|e| format!("Failed to write temporary target '{}': {e}", tmp.display()))?;
    fs::rename(&tmp, target).map_err(|e| {
        let _ = fs::remove_file(&tmp);
        format!(
            "Failed to atomically replace '{}' with '{}': {e}",
            target.display(),
            tmp.display()
        )
    })
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
