//! Marker block source — obtains Marker's block-tree JSON and parses it to `Vec<Block>`.
//!
//! Device-agnostic by construction: Marker runs as a Python sidecar that auto-detects its
//! device (cuda → mps → cpu); the transport here is **orthogonal** to device —
//! a local subprocess (default, the standalone-Mac story), a remote HTTP Marker service
//! (e.g. WRAITH for bulk on NVIDIA), or a pre-extracted JSON file you produce out of band.
//! All three yield the same block-tree JSON, parsed by
//! `archon_ingest_ext::marker::parse_marker_str`.

use std::path::{Path, PathBuf};

use archon_ingest_ext::chunk::Block;
use archon_ingest_ext::marker::parse_marker_str;
use archon_policy::PdfPolicy;

use crate::errors::DocsError;

/// Where/how to obtain a PDF's Marker block tree.
#[derive(Clone, Debug)]
pub enum MarkerSource {
    /// Spawn the local Python sidecar: `python <script> <pdf> [--device <dev>]`.
    /// `device = None` lets the sidecar auto-detect (cuda → mps → cpu).
    Subprocess {
        python: String,
        script: PathBuf,
        device: Option<String>,
    },
    /// POST the PDF bytes to a remote Marker HTTP service; expects block-tree JSON back.
    Http { url: String },
    /// Read a pre-extracted Marker JSON file (decoupled — you run Marker however/whenever).
    PreExtracted { json_path: PathBuf },
}

/// Build a `MarkerSource` from policy: present `marker_sidecar` path → local subprocess
/// (with optional `marker_device`); absent → `None` (caller falls back to flat-text blocks).
pub fn from_policy(pdf: &PdfPolicy) -> Option<MarkerSource> {
    pdf.marker_sidecar.as_ref().map(|script| MarkerSource::Subprocess {
        python: "python3".to_string(),
        script: PathBuf::from(script),
        device: pdf.marker_device.clone(),
    })
}

impl MarkerSource {
    /// Obtain and parse the Marker block stream for `pdf_path`.
    pub async fn blocks_for(&self, pdf_path: &Path) -> Result<Vec<Block>, DocsError> {
        let json = self.fetch_json(pdf_path).await?;
        parse_marker_str(&json).map_err(|e| DocsError::Storage {
            message: format!("marker json parse failed: {e}"),
        })
    }

    async fn fetch_json(&self, pdf_path: &Path) -> Result<String, DocsError> {
        match self {
            MarkerSource::Subprocess { python, script, device } => {
                let mut cmd = tokio::process::Command::new(python);
                cmd.arg(script).arg(pdf_path);
                if let Some(dev) = device {
                    cmd.arg("--device").arg(dev);
                }
                let out = cmd.output().await.map_err(|e| DocsError::Storage {
                    message: format!("marker sidecar spawn failed: {e}"),
                })?;
                if !out.status.success() {
                    return Err(DocsError::Storage {
                        message: format!(
                            "marker sidecar exited {}: {}",
                            out.status,
                            String::from_utf8_lossy(&out.stderr)
                        ),
                    });
                }
                String::from_utf8(out.stdout).map_err(|e| DocsError::Storage {
                    message: format!("marker sidecar stdout not utf-8: {e}"),
                })
            }
            MarkerSource::Http { url } => {
                let bytes = tokio::fs::read(pdf_path).await.map_err(|e| DocsError::Storage {
                    message: format!("read pdf for marker http failed: {e}"),
                })?;
                let resp = reqwest::Client::new()
                    .post(url)
                    .body(bytes)
                    .send()
                    .await
                    .map_err(|e| DocsError::Storage {
                        message: format!("marker http request failed: {e}"),
                    })?;
                resp.text().await.map_err(|e| DocsError::Storage {
                    message: format!("marker http response read failed: {e}"),
                })
            }
            MarkerSource::PreExtracted { json_path } => {
                tokio::fs::read_to_string(json_path).await.map_err(|e| DocsError::Storage {
                    message: format!("read pre-extracted marker json failed: {e}"),
                })
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use archon_ingest_ext::chunk::BlockType;

    #[tokio::test]
    async fn pre_extracted_source_parses_block_tree() {
        let json = r#"{
            "block_type": "Document",
            "children": [{
                "block_type": "Page", "id": "/page/0/Page/0",
                "children": [
                    {"block_type": "SectionHeader", "html": "<h1>Title</h1>", "bbox": [1,2,3,4]},
                    {"block_type": "Text", "html": "<p>Body.</p>", "bbox": [1,5,3,8]}
                ]
            }]
        }"#;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("doc.marker.json");
        std::fs::write(&path, json).unwrap();

        let src = MarkerSource::PreExtracted { json_path: path };
        let blocks = src.blocks_for(Path::new("ignored.pdf")).await.unwrap();
        assert_eq!(blocks.len(), 2);
        assert_eq!(blocks[0].block_type, BlockType::SectionHeader);
        assert_eq!(blocks[0].text, "Title");
        assert_eq!(blocks[1].text, "Body.");
    }

    #[tokio::test]
    async fn subprocess_failure_surfaces_as_error() {
        // A non-existent script → spawn/exec error, surfaced (the pipeline catches this and
        // falls back to flat-text blocks).
        let src = MarkerSource::Subprocess {
            python: "python3".into(),
            script: PathBuf::from("/nonexistent/archon_marker_sidecar.py"),
            device: Some("cpu".into()),
        };
        assert!(src.blocks_for(Path::new("x.pdf")).await.is_err());
    }

    #[test]
    fn from_policy_maps_sidecar_path() {
        let mut pdf = PdfPolicy::default();
        assert!(from_policy(&pdf).is_none(), "no sidecar → None");
        pdf.marker_sidecar = Some("scripts/archon_marker_sidecar.py".into());
        pdf.marker_device = Some("mps".into());
        match from_policy(&pdf) {
            Some(MarkerSource::Subprocess { device, .. }) => assert_eq!(device.as_deref(), Some("mps")),
            other => panic!("expected subprocess, got {other:?}"),
        }
    }
}
