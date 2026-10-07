//! Host-written author context (Issue 288).
//!
//! An author prompt used to carry every earlier result in full: a live
//! acceptance author prompt reached 455 KB, 436 KB of it the entries already
//! completed. A prompt now names each earlier result by a one-line record and
//! the SHA-256 of its exact bytes, and the bytes themselves live in a file the
//! host writes here, under the run's own directory, named by that digest.
//!
//! `__archonAuthorContext(extension, text)` writes `text` once to
//! `<dir>/<sha256>.<extension>` and returns `{"path", "sha256"}`. The name is
//! the content, so the same text always resolves to the same path (a resumed
//! run makes the same prompts), and a file that already holds other bytes is
//! a fault, never overwritten. A dry run computes the same digest and writes
//! nothing: its prompts are never dispatched.

use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

/// The directory under a run directory that holds its author context.
pub const AUTHOR_CONTEXT_DIR: &str = "author-context";

/// Extensions a context file may take: what the script writes.
const EXTENSIONS: [&str; 3] = ["json", "jsonl", "txt"];

/// The author-context directory of the run whose directory is `run_dir`.
pub fn author_context_dir(run_dir: &Path) -> PathBuf {
    run_dir.join(AUTHOR_CONTEXT_DIR)
}

/// Where the binding puts what it is given.
#[derive(Debug, Clone)]
pub enum AuthorContextStore {
    /// A live run: files are written under this directory.
    Write(PathBuf),
    /// A dry run: the digest is computed, nothing is written.
    Preview,
}

/// The lowercase hex SHA-256 of `bytes`.
pub fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// Write `text` once under `dir`, named by its digest. Returns the path and
/// the digest. An existing file is compared byte for byte and never replaced.
pub fn write_author_context(
    dir: &Path,
    extension: &str,
    text: &str,
) -> Result<(PathBuf, String), String> {
    let (name, digest) = file_name(extension, text)?;
    let path = dir.join(&name);
    match std::fs::read(&path) {
        Ok(existing) if existing == text.as_bytes() => return Ok((path, digest)),
        Ok(_) => {
            return Err(format!(
                "author context {} holds other bytes than its name: the file was changed after the host wrote it",
                path.display()
            ));
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => {
            return Err(format!(
                "reading author context {}: {error}",
                path.display()
            ));
        }
    }
    std::fs::create_dir_all(dir).map_err(|error| {
        format!(
            "creating author context directory {}: {error}",
            dir.display()
        )
    })?;
    // Written beside its final name and renamed into place, so a reader never
    // sees a partial file and two writers of the same text both succeed.
    let staged = dir.join(format!(
        ".{name}.{}.{}.tmp",
        std::process::id(),
        STAGE.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    ));
    std::fs::write(&staged, text.as_bytes())
        .map_err(|error| format!("writing author context {}: {error}", staged.display()))?;
    std::fs::rename(&staged, &path).map_err(|error| {
        let _ = std::fs::remove_file(&staged);
        format!("publishing author context {}: {error}", path.display())
    })?;
    Ok((path, digest))
}

static STAGE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

fn file_name(extension: &str, text: &str) -> Result<(String, String), String> {
    if !EXTENSIONS.contains(&extension) {
        return Err(format!(
            "author context extension '{extension}' is not one of {}",
            EXTENSIONS.join(", ")
        ));
    }
    let digest = sha256_hex(text.as_bytes());
    Ok((format!("{digest}.{extension}"), digest))
}

/// What one binding call returns to the script.
fn resolve(store: &AuthorContextStore, extension: &str, text: &str) -> Result<String, String> {
    let (path, digest) = match store {
        AuthorContextStore::Write(dir) => {
            let (path, digest) = write_author_context(dir, extension, text)?;
            (path.to_string_lossy().replace('\\', "/"), digest)
        }
        AuthorContextStore::Preview => {
            let (name, digest) = file_name(extension, text)?;
            (format!("{AUTHOR_CONTEXT_DIR}/{name}"), digest)
        }
    };
    serde_json::to_string(&serde_json::json!({ "path": path, "sha256": digest }))
        .map_err(|error| error.to_string())
}

/// Installs `__archonAuthorContext(extension, text)` into `ctx`. A write the
/// host cannot make throws: the script reports it, never a reference to
/// bytes that are not there.
pub fn install_author_context(
    ctx: &rquickjs::Ctx<'_>,
    store: AuthorContextStore,
) -> rquickjs::Result<()> {
    ctx.globals().set(
        "__archonAuthorContext",
        rquickjs::function::Func::from(
            move |extension: String, text: String| -> rquickjs::Result<String> {
                resolve(&store, &extension, &text).map_err(|message| {
                    rquickjs::Error::new_from_js_message("author context", "string", message)
                })
            },
        ),
    )
}

/// The live binding of the run whose directory is `run_dir` (Issue 288).
pub fn install_for_run(ctx: &rquickjs::Ctx<'_>, run_dir: &Path) -> rquickjs::Result<()> {
    install_author_context(ctx, AuthorContextStore::Write(author_context_dir(run_dir)))
}

/// The dry-run binding: digests computed, nothing written.
pub fn install_preview(ctx: &rquickjs::Ctx<'_>) -> rquickjs::Result<()> {
    install_author_context(ctx, AuthorContextStore::Preview)
}

#[cfg(test)]
#[path = "author_context_tests.rs"]
mod tests;
