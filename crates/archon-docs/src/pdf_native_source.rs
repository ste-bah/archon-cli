//! Native-coordinate block source for born-digital PDFs.
//!
//! Runs `scripts/archon_pdf_native_sidecar.py` (pdftotext -tsv glyph positions — no OCR,
//! no GPU, sub-second per document) and parses its stdout with the SAME
//! `archon_ingest_ext::marker::parse_marker_str` the Marker path uses: the sidecar emits
//! Marker-compatible block-tree JSON, so no new Rust parser exists. Blocks land in the
//! chunker under `COORD_PDF_NATIVE` — identical spatial persistence to Marker, distinct
//! coord-space string so the extraction method stays queryable.
//!
//! Failure is never fatal to an ingest: the caller (ingest_pdf routing) falls back to
//! Marker (if configured) or the flat pdftotext path on any error here.

use std::path::{Path, PathBuf};

use archon_ingest_ext::chunk::Block;
use archon_ingest_ext::marker::parse_marker_str;
use archon_policy::PdfPolicy;

use crate::errors::DocsError;

/// Wall-clock bound on one sidecar run. Native extraction is pdftotext-fast (a 300-page
/// book measures ~2s), so minutes of headroom means an elapse is a genuine hang.
fn native_timeout() -> std::time::Duration {
    let secs = std::env::var("ARCHON_PDF_NATIVE_TIMEOUT_SECS")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(300);
    std::time::Duration::from_secs(secs)
}

/// Obtains native-coordinate blocks from a born-digital PDF via the Python sidecar.
#[derive(Clone, Debug)]
pub struct PdfNativeSource {
    python: String,
    script: PathBuf,
}

impl PdfNativeSource {
    /// Build from policy. Returns `None` (feature off / not resolvable) when
    /// `use_pdf_native_extractor = false` or no sidecar script can be located.
    ///
    /// Script resolution order: `pdf_native_script` policy → `ARCHON_PDF_NATIVE_SCRIPT`
    /// env → `scripts/archon_pdf_native_sidecar.py` next to the archon binary → the same
    /// path under the current directory (the from-checkout case).
    pub fn from_policy(pdf: &PdfPolicy) -> Option<Self> {
        if !pdf.use_pdf_native_extractor {
            return None;
        }
        let script = resolve_script(pdf.pdf_native_script.as_deref())?;
        Some(Self {
            python: pdf
                .pdf_native_python
                .clone()
                .unwrap_or_else(|| "python3".to_string()),
            script,
        })
    }

    /// Run the sidecar for `path` and parse its stdout. `Ok(None)` = the sidecar ran but
    /// found no text blocks (image-only PDF pretending to be born-digital); `Err` = the
    /// sidecar is missing/misconfigured/crashed — the caller falls back either way.
    pub async fn blocks_for(&self, path: &Path) -> Result<Option<Vec<Block>>, DocsError> {
        let mut cmd = tokio::process::Command::new(&self.python);
        cmd.arg(&self.script).arg(path);
        cmd.kill_on_drop(true);
        let out = match tokio::time::timeout(native_timeout(), cmd.output()).await {
            Err(_elapsed) => {
                return Err(DocsError::Storage {
                    message: format!(
                        "pdf-native sidecar timed out after {}s",
                        native_timeout().as_secs()
                    ),
                });
            }
            Ok(res) => res.map_err(|e| DocsError::Storage {
                message: format!("pdf-native sidecar spawn failed: {e}"),
            })?,
        };
        if !out.status.success() {
            let stderr = String::from_utf8_lossy(&out.stderr);
            return Err(DocsError::Storage {
                message: format!(
                    "pdf-native sidecar exited {}: {}",
                    out.status,
                    stderr.trim()
                ),
            });
        }
        let json = String::from_utf8(out.stdout).map_err(|e| DocsError::Storage {
            message: format!("pdf-native sidecar stdout not utf-8: {e}"),
        })?;
        let blocks = parse_marker_str(&json).map_err(|e| DocsError::Storage {
            message: format!("pdf-native json parse failed: {e}"),
        })?;
        if blocks.is_empty() {
            return Ok(None);
        }
        Ok(Some(blocks))
    }
}

/// Locate the sidecar script (see [`PdfNativeSource::from_policy`] for the order).
fn resolve_script(policy_path: Option<&str>) -> Option<PathBuf> {
    if let Some(p) = policy_path {
        let pb = PathBuf::from(p);
        return pb.is_file().then_some(pb);
    }
    if let Ok(p) = std::env::var("ARCHON_PDF_NATIVE_SCRIPT") {
        let pb = PathBuf::from(p);
        if pb.is_file() {
            return Some(pb);
        }
    }
    const REL: &str = "scripts/archon_pdf_native_sidecar.py";
    if let Ok(exe) = std::env::current_exe() {
        // target/{debug,release}/archon → repo root two levels up; also try the exe's dir
        // itself for installed layouts that ship scripts/ alongside the binary.
        for base in exe.ancestors().skip(1).take(4) {
            let cand = base.join(REL);
            if cand.is_file() {
                return Some(cand);
            }
        }
    }
    let cwd = PathBuf::from(REL);
    cwd.is_file().then_some(cwd)
}

#[cfg(test)]
#[path = "pdf_native_source_tests.rs"]
mod tests;
