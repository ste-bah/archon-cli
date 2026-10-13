//! Candidate stdin past a capability's inline bound (Issue 386, item 3).
//!
//! A capability's `max_stdin_bytes` is the most it takes inline through its
//! pipe; it is not a reason to fail the run. The host writes a larger
//! candidate, byte for byte, to `stdin.bin` in the attempt's spill directory
//! under the run (`host-command-results/<call>/attempt-*/`), and the child's
//! stdin carries a bounded view instead: one line that names the spill and
//! its length and digest. The child reads the full candidate from the spill,
//! never the view. Authoritative run state keeps host-command stdin as
//! authored (`archon_workflow::events::REDACTION_MARKER`), so the spill is
//! not redacted either: it holds exactly the bytes an inline pipe would.
//!
//! The view's first bytes are reserved. An inline candidate that begins with
//! them is spilled too, so the child never takes authored text for a view.
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use archon_workflow::{WorkflowError, WorkflowResult};

/// The reserved first bytes of a spill view.
pub(crate) const STDIN_SPILL_PREFIX: &str = "archon-host-stdin-spill/v1 ";

/// The longest view a child reads: a view names one absolute path, a
/// length and a digest.
const MAX_VIEW_BYTES: usize = 64 * 1024;

/// The file a spilled candidate is written to, in the attempt's directory.
pub(crate) const STDIN_SPILL_FILE: &str = "stdin.bin";

/// What a view names: the spill and the identity of its bytes.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct StdinSpillView {
    pub(crate) path: PathBuf,
    pub(crate) bytes: u64,
    pub(crate) blake3: String,
}

impl StdinSpillView {
    fn encode(&self) -> WorkflowResult<Vec<u8>> {
        let mut line = STDIN_SPILL_PREFIX.as_bytes().to_vec();
        line.extend_from_slice(&serde_json::to_vec(self)?);
        line.push(b'\n');
        Ok(line)
    }
}

/// Whether `stdin` goes to the child through a spill: longer than `limit`,
/// or beginning with the reserved view prefix.
pub(crate) fn needs_spill(stdin: &[u8], limit: u64) -> bool {
    stdin.len() as u64 > limit || stdin.starts_with(STDIN_SPILL_PREFIX.as_bytes())
}

/// Spill `stdin` to `directory` (the attempt's spill directory under the
/// run) and return the bounded view the child reads instead.
pub(crate) fn spill(directory: &Path, stdin: &[u8]) -> WorkflowResult<Vec<u8>> {
    crate::command::workflow_host_command_supervisor::create_spill_directories(directory)?;
    let path = directory.join(STDIN_SPILL_FILE);
    let io = |source| WorkflowError::Io {
        path: path.clone(),
        source,
    };
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW).mode(0o600);
    }
    let mut file = options.open(&path).map_err(io)?;
    file.write_all(stdin).map_err(io)?;
    file.sync_all().map_err(io)?;
    StdinSpillView {
        path,
        bytes: stdin.len() as u64,
        blake3: archon_workflow::task_set_contract::content_digest(stdin),
    }
    .encode()
}

/// The trusted child's candidate: its stdin, or the spill a view on it
/// names. Inline stdin longer than `limit` is refused as before. A view is
/// honoured only for a regular file under this call's run
/// (`<staging_root>/../../host-command-results/`), whose length and digest
/// are the ones the view names.
pub(crate) fn read_candidate(
    mut stdin: impl Read,
    limit: usize,
    staging_root: &Path,
) -> anyhow::Result<Vec<u8>> {
    use anyhow::{Context, anyhow};
    let mut bytes = Vec::new();
    // Enough to tell a view's prefix even under a shorter inline limit.
    let first = limit.max(STDIN_SPILL_PREFIX.len()) as u64 + 1;
    (&mut stdin)
        .take(first)
        .read_to_end(&mut bytes)
        .context("reading trusted candidate stdin")?;
    if !bytes.starts_with(STDIN_SPILL_PREFIX.as_bytes()) {
        if bytes.len() > limit {
            return Err(anyhow!("candidate stdin exceeds {limit} bytes"));
        }
        return Ok(bytes);
    }
    // A view is one short line, whatever the inline limit: read all of it.
    stdin
        .take(MAX_VIEW_BYTES as u64)
        .read_to_end(&mut bytes)
        .context("reading trusted candidate stdin spill view")?;
    if bytes.len() > MAX_VIEW_BYTES {
        return Err(anyhow!(
            "candidate stdin spill view exceeds {MAX_VIEW_BYTES} bytes"
        ));
    }
    let view = &bytes[STDIN_SPILL_PREFIX.len()..];
    let view: StdinSpillView = serde_json::from_slice(view.trim_ascii_end())
        .context("candidate stdin spill view is malformed")?;
    let results = staging_root
        .parent()
        .and_then(Path::parent)
        .map(|run| run.join("host-command-results"))
        .ok_or_else(|| {
            anyhow!(
                "staging root {} has no run directory for a stdin spill",
                staging_root.display()
            )
        })?;
    let canonical_results = results
        .canonicalize()
        .with_context(|| format!("resolving stdin spill root {}", results.display()))?;
    let canonical = view
        .path
        .canonicalize()
        .with_context(|| format!("resolving stdin spill {}", view.path.display()))?;
    let metadata = std::fs::symlink_metadata(&view.path)
        .with_context(|| format!("inspecting stdin spill {}", view.path.display()))?;
    if !view.path.is_absolute()
        || !canonical.starts_with(&canonical_results)
        || !metadata.file_type().is_file()
    {
        return Err(anyhow!(
            "candidate stdin spill {} is not a regular file under {}",
            view.path.display(),
            results.display()
        ));
    }
    let mut options = std::fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW);
    }
    let mut full = Vec::new();
    options
        .open(&view.path)
        .and_then(|mut file| file.read_to_end(&mut full))
        .with_context(|| format!("reading stdin spill {}", view.path.display()))?;
    let digest = archon_workflow::task_set_contract::content_digest(&full);
    if full.len() as u64 != view.bytes || digest != view.blake3 {
        return Err(anyhow!(
            "candidate stdin spill {} holds {} bytes digesting to {digest}, but its view names {} bytes digesting to {}",
            view.path.display(),
            full.len(),
            view.bytes,
            view.blake3
        ));
    }
    Ok(full)
}

#[cfg(test)]
#[path = "workflow_host_command_stdin_spill_tests.rs"]
mod tests;
