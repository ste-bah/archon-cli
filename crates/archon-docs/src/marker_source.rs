//! Marker block source — obtains Marker's block-tree JSON and parses it to `Vec<Block>`.
//!
//! Device-adaptive: the Marker sidecar runs as a Python subprocess, but WHICH device it runs on
//! (and at what surya batch caps) is resolved by `archon-accel` from the host's *free* VRAM — a
//! bigger card gets bigger batches. The load-bearing correctness guarantee is the per-doc
//! OOM→retry-smaller ladder in `fetch_json`: step DOWN GPU batch tiers, CPU as the last resort.
//! Transport is orthogonal to device: a local
//! subprocess (default, the standalone Mac/laptop story), a remote HTTP Marker service (e.g.
//! WRAITH for bulk on NVIDIA), or a pre-extracted JSON file. All three yield the same
//! block-tree JSON, parsed by `archon_ingest_ext::marker::parse_marker_str`.

use std::path::{Path, PathBuf};

use archon_accel::{AccelKind, DeviceOverrides, marker_env_ladder};
use archon_ingest_ext::chunk::Block;
use archon_ingest_ext::marker::parse_marker_str;
use archon_policy::PdfPolicy;

use crate::errors::DocsError;

/// Where/how to obtain a PDF's Marker block tree.
#[derive(Clone, Debug)]
pub enum MarkerSource {
    /// Spawn the local Python sidecar per the `archon-accel` OOM→retry-smaller ladder — each
    /// attempt runs `python <script> <pdf> --device <dev>` under that attempt's env.
    Subprocess {
        python: String,
        script: PathBuf,
        /// `(sidecar_device, env)` per attempt, biggest batch tier first, CPU last. Tried
        /// top-to-bottom, advancing to the next (smaller) attempt only on a torch-OOM.
        attempts: Vec<(String, Vec<(String, String)>)>,
    },
    /// POST the PDF bytes to a remote Marker HTTP service; expects block-tree JSON back.
    Http { url: String },
    /// Read a pre-extracted Marker JSON file (decoupled — you run Marker however/whenever).
    PreExtracted { json_path: PathBuf },
}

/// Build a `MarkerSource` from policy. Present `marker_sidecar` path → local subprocess whose
/// device + surya batch caps + OOM→retry-smaller ladder are resolved by `archon-accel` from the
/// host's free VRAM (`marker_device` of `None`/`"auto"` → planner-chosen; an explicit
/// `cuda|mps|cpu` forces it). Absent `marker_sidecar` → `None` (caller falls back to flat text).
///
/// NOTE: resolves placement (a cheap `nvidia-smi`/`sysinfo` probe) once per call — in bulk ingest,
/// once per PDF. It can be hoisted to once-per-run if the probe ever shows up in a profile.
pub fn from_policy(pdf: &PdfPolicy) -> Option<MarkerSource> {
    let script = pdf.marker_sidecar.as_ref()?;
    let attempts = marker_env_ladder(&archon_accel::detect(), &overrides_from_policy(pdf));
    Some(MarkerSource::Subprocess {
        python: pdf
            .marker_python
            .clone()
            .unwrap_or_else(|| "python3".to_string()),
        script: PathBuf::from(script),
        attempts,
    })
}

/// Map the policy's `marker_device` knob to `archon-accel` overrides. `None`/`"auto"`/unknown →
/// planner auto-detect; an explicit `cuda|mps|metal|cpu` forces the device (still OOM-guarded).
fn overrides_from_policy(pdf: &PdfPolicy) -> DeviceOverrides {
    let force_marker_device = match pdf.marker_device.as_deref() {
        None | Some("auto") | Some("") => None,
        Some("cuda") => Some(AccelKind::Cuda),
        Some("mps") | Some("metal") => Some(AccelKind::Metal),
        Some("cpu") => Some(AccelKind::Cpu),
        Some(_) => None,
    };
    DeviceOverrides {
        force_marker_device,
        memory_budget_mb: pdf.marker_memory_budget_mb,
        ..DeviceOverrides::default()
    }
}

/// Internal: distinguishes a torch-OOM sidecar failure (advance the ladder) from any other error.
enum SidecarError {
    Oom { device: String },
    Other(DocsError),
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
            MarkerSource::Subprocess {
                python,
                script,
                attempts,
            } => {
                // Try each ladder rung; advance to the next (smaller) attempt only on torch-OOM.
                let mut last: Option<DocsError> = None;
                for (device, env) in attempts {
                    match run_sidecar(python, script, pdf_path, Some(device.as_str()), env).await {
                        Ok(json) => return Ok(json),
                        Err(SidecarError::Oom { device: dev }) => {
                            last = Some(DocsError::Storage {
                                message: format!("marker OOM on device={dev}"),
                            });
                        }
                        Err(SidecarError::Other(e)) => return Err(e),
                    }
                }
                Err(last.unwrap_or_else(|| DocsError::Storage {
                    message: "marker: empty attempt ladder".to_string(),
                }))
            }
            MarkerSource::Http { url } => {
                let bytes = tokio::fs::read(pdf_path)
                    .await
                    .map_err(|e| DocsError::Storage {
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
            MarkerSource::PreExtracted { json_path } => tokio::fs::read_to_string(json_path)
                .await
                .map_err(|e| DocsError::Storage {
                    message: format!("read pre-extracted marker json failed: {e}"),
                }),
        }
    }
}

/// Run the Marker sidecar once with `device` + `env`, classifying a torch-OOM exit (the sidecar
/// exits 42 on OOM, or carries an OOM signature on stderr) so the caller can retry on CPU.
async fn run_sidecar(
    python: &str,
    script: &Path,
    pdf_path: &Path,
    device: Option<&str>,
    env: &[(String, String)],
) -> Result<String, SidecarError> {
    let mut cmd = tokio::process::Command::new(python);
    cmd.arg(script).arg(pdf_path);
    if let Some(dev) = device {
        cmd.arg("--device").arg(dev);
    }
    for (k, v) in env {
        cmd.env(k, v);
    }
    let out = cmd.output().await.map_err(|e| {
        SidecarError::Other(DocsError::Storage {
            message: format!("marker sidecar spawn failed: {e}"),
        })
    })?;
    if out.status.success() {
        return String::from_utf8(out.stdout).map_err(|e| {
            SidecarError::Other(DocsError::Storage {
                message: format!("marker sidecar stdout not utf-8: {e}"),
            })
        });
    }
    let stderr = String::from_utf8_lossy(&out.stderr);
    let is_oom = out.status.code() == Some(42)
        || stderr.to_lowercase().contains("out of memory")
        || stderr.contains("OutOfMemoryError");
    if is_oom {
        return Err(SidecarError::Oom {
            device: device.unwrap_or("cpu").to_string(),
        });
    }
    Err(SidecarError::Other(DocsError::Storage {
        message: format!("marker sidecar exited {}: {}", out.status, stderr),
    }))
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
            attempts: vec![("cpu".to_string(), vec![])],
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
            Some(MarkerSource::Subprocess { attempts, .. }) => {
                assert_eq!(attempts.first().unwrap().0.as_str(), "mps")
            }
            other => panic!("expected subprocess, got {other:?}"),
        }
    }

    #[test]
    fn overrides_map_explicit_devices() {
        let mut p = PdfPolicy::default();
        assert_eq!(overrides_from_policy(&p).force_marker_device, None);
        p.marker_device = Some("auto".into());
        assert_eq!(overrides_from_policy(&p).force_marker_device, None);
        p.marker_device = Some("cuda".into());
        assert_eq!(
            overrides_from_policy(&p).force_marker_device,
            Some(AccelKind::Cuda)
        );
        p.marker_device = Some("mps".into());
        assert_eq!(
            overrides_from_policy(&p).force_marker_device,
            Some(AccelKind::Metal)
        );
        p.marker_device = Some("cpu".into());
        assert_eq!(
            overrides_from_policy(&p).force_marker_device,
            Some(AccelKind::Cpu)
        );
    }

    #[test]
    fn from_policy_forced_cpu_yields_cpu_device_and_env() {
        // Host-agnostic: forcing cpu must produce a cpu sidecar device + a non-empty env,
        // regardless of what hardware `detect()` finds on the test host.
        let p = PdfPolicy {
            marker_sidecar: Some("scripts/archon_marker_sidecar.py".into()),
            marker_device: Some("cpu".into()),
            ..Default::default()
        };
        match from_policy(&p) {
            Some(MarkerSource::Subprocess { attempts, .. }) => {
                let (device, env) = attempts.first().unwrap();
                assert_eq!(device.as_str(), "cpu");
                assert!(env.iter().any(|(k, _)| k == "TORCH_DEVICE"));
            }
            other => panic!("expected subprocess, got {other:?}"),
        }
    }
}
