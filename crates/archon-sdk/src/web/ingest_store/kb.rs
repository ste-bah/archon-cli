//! Knowledge bases for the web ingest tab.
//!
//! "Knowledge base" used to mean two unconnected things: a directory under
//! `.archon/kb/` to the web workbench, and a `kb_id` string in the store to
//! the CLI and TUI. Neither surface read the other's storage, so a knowledge
//! base created on one side was invisible on the other with no error (#170).
//!
//! [`knowledge_bases`] returns the union of both. Origin is reported on each
//! row but never decides whether a row is listed: a knowledge base is listed
//! because it exists, not because of how it came to.
//!
//! ## Working directory
//!
//! The directory roots are resolved from `paths.cwd`, the server's working
//! directory, not from a discovered project root — so a server launched below
//! the project root reads a different `.archon/kb`. That is deliberate here:
//! the CLI resolves its own store the same way
//! (`command::store_paths::evidence_db_path` joins `.archon` onto the process
//! working directory with no upward walk), and teaching only the web tab to
//! walk up would make the two surfaces disagree about which project they are
//! looking at — the same class of split this module exists to close. A project
//! root walk belongs in the shared path resolution every surface uses, in one
//! change.

use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};

use anyhow::Result;
use cozo::DbInstance;

use super::{WebRuntimePaths, dir_stats};
use crate::web::ingest::{WebKbCreateRequest, WebKnowledgeBaseItem};

/// A knowledge base recorded in the store only.
const ORIGIN_DB: &str = "db";
/// A knowledge base that exists only as a directory.
const ORIGIN_DIR: &str = "dir";
/// A knowledge base recorded in both places.
const ORIGIN_BOTH: &str = "both";

/// Every knowledge base, from the store and from the filesystem, merged.
///
/// `warnings` collects what could not be read. A caller must render those:
/// an unreadable store and a store with no knowledge bases both produce an
/// empty list, and the two must not look the same.
pub(super) fn knowledge_bases(
    paths: &WebRuntimePaths,
    db: Option<&DbInstance>,
    warnings: &mut Vec<String>,
) -> Vec<WebKnowledgeBaseItem> {
    let mut merged: BTreeMap<String, WebKnowledgeBaseItem> = BTreeMap::new();
    for (scope, root) in dir_roots(paths) {
        for item in kb_dirs(scope, &root, warnings) {
            merge_directory(&mut merged, item);
        }
    }
    merge_store(&mut merged, db, warnings);
    merged.into_values().collect()
}

/// Create a knowledge base from the web workbench.
///
/// The name is registered in the store *before* the directory is created. The
/// registration is what every other surface reads — `--kb` matches the stored
/// `kb_id`, never the directory slug — so a run that made the directory and
/// failed to register would rebuild the split this change removes.
pub(crate) fn create_kb(
    paths: &WebRuntimePaths,
    request: &WebKbCreateRequest,
) -> Result<WebKnowledgeBaseItem> {
    let name = request.name.trim();
    if name.is_empty() {
        anyhow::bail!("knowledge base name is required");
    }
    let scope = if request.scope == "home" {
        "home"
    } else {
        "project"
    };
    let description = request
        .description
        .as_deref()
        .unwrap_or("Knowledge base notes.");

    let db = super::open_docs_db(paths)?;
    archon_knowledge::schema::ensure_knowledge_schema(&db)?;
    archon_knowledge::store::register_kb(&db, name, scope, description)?;

    let root = if scope == "home" {
        home_archon().join("kb")
    } else {
        paths.cwd.join(".archon/kb")
    };
    let dir = root.join(slugify(name));
    fs::create_dir_all(&dir)?;
    let readme = dir.join("README.md");
    if !readme.exists() {
        fs::write(&readme, format!("# {name}\n\n{description}\n"))?;
    }
    let mut item = directory_item(name, scope, &dir);
    item.origin = ORIGIN_BOTH.into();
    Ok(item)
}

fn dir_roots(paths: &WebRuntimePaths) -> [(&'static str, PathBuf); 2] {
    [
        ("project", paths.cwd.join(".archon/kb")),
        ("home", home_archon().join("kb")),
    ]
}

/// Directories under one root.
///
/// A root that does not exist is ordinary — most projects never create one.
/// A root that exists and cannot be read is not, and says so.
fn kb_dirs(scope: &str, root: &Path, warnings: &mut Vec<String>) -> Vec<WebKnowledgeBaseItem> {
    let entries = match fs::read_dir(root) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Vec::new(),
        Err(error) => {
            warnings.push(format!(
                "knowledge base directory {} could not be read: {error}",
                root.display()
            ));
            return Vec::new();
        }
    };
    entries
        .flatten()
        .filter_map(|entry| {
            entry.metadata().ok()?.is_dir().then(|| {
                let name = entry.file_name().to_string_lossy().to_string();
                directory_item(&name, scope, &entry.path())
            })
        })
        .collect()
}

fn directory_item(name: &str, scope: &str, path: &Path) -> WebKnowledgeBaseItem {
    let (files, bytes) = dir_stats(path, 0);
    WebKnowledgeBaseItem {
        name: name.into(),
        scope: scope.into(),
        path: path.to_string_lossy().to_string(),
        files,
        bytes,
        exists: path.exists(),
        origin: ORIGIN_DIR.into(),
        documents: 0,
    }
}

/// Add a directory row.
///
/// `--kb` is a flat namespace with no notion of scope, so a slug that appears
/// under both the project and the home root is one knowledge base as far as
/// any command line is concerned. Merging keeps that honest — and keeps both
/// scopes named on the row instead of dropping one.
fn merge_directory(
    merged: &mut BTreeMap<String, WebKnowledgeBaseItem>,
    item: WebKnowledgeBaseItem,
) {
    let key = slugify(&item.name);
    match merged.get_mut(&key) {
        Some(existing) => {
            existing.scope = format!("{}+{}", existing.scope, item.scope);
            existing.files += item.files;
            existing.bytes += item.bytes;
        }
        None => {
            merged.insert(key, item);
        }
    }
}

fn merge_store(
    merged: &mut BTreeMap<String, WebKnowledgeBaseItem>,
    db: Option<&DbInstance>,
    warnings: &mut Vec<String>,
) {
    let Some(db) = db else {
        warnings.push(
            "knowledge bases held in the document store could not be listed: the store is not readable"
                .into(),
        );
        return;
    };
    let rows = match archon_knowledge::store::list_kbs(db) {
        Ok(rows) => rows,
        Err(error) => {
            warnings.push(format!(
                "knowledge base listing from the document store failed: {error}"
            ));
            return;
        }
    };
    for row in rows {
        let key = slugify(&row.kb_id);
        match merged.get_mut(&key) {
            Some(existing) => {
                // The stored `kb_id` wins over the directory slug: `--kb`
                // matches it exactly, so it is the string the reader needs.
                existing.name = row.kb_id;
                existing.origin = ORIGIN_BOTH.into();
                existing.documents = row.documents;
            }
            None => {
                merged.insert(
                    key,
                    WebKnowledgeBaseItem {
                        name: row.kb_id,
                        scope: if row.scope.is_empty() {
                            "store".into()
                        } else {
                            row.scope
                        },
                        path: String::new(),
                        files: 0,
                        bytes: 0,
                        exists: false,
                        origin: ORIGIN_DB.into(),
                        documents: row.documents,
                    },
                );
            }
        }
    }
}

fn slugify(name: &str) -> String {
    let mut out = String::new();
    for ch in name.chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch.to_ascii_lowercase());
        } else if matches!(ch, ' ' | '-' | '_') && !out.ends_with('-') {
            out.push('-');
        }
    }
    out.trim_matches('-').to_string()
}

fn home_archon() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".archon")
}

#[cfg(test)]
#[path = "kb_tests.rs"]
mod tests;
