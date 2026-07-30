//! Marker block source — obtains Marker's block-tree JSON and parses it to `Vec<Block>`.
//!
//! Device-adaptive: the Marker sidecar runs as a Python subprocess; `archon-accel` plans one or
//! more *chunks* from the host's *free* VRAM vs the document's page-scaled footprint (Marker's VRAM
//! tracks document size, not batch — PR-D). A document that won't fit a small card whole is split
//! into contiguous page-range chunks (`--page-range`) that each fit; Marker emits absolute page
//! ids, so the chunks' block streams concatenate in order without re-offset. Each chunk carries a
//! per-chunk OOM→CPU fallback (`run_chunk`: a GPU rung, then CPU — smaller batches don't relieve
//! OOM). Transport is orthogonal to device: a local subprocess (default, the standalone Mac/laptop
//! story), a persistent HTTP Marker server (warm resident models for bulk ingest), or a
//! pre-extracted JSON file. All three yield the same block-tree JSON, parsed by
//! `archon_ingest_ext::marker::parse_marker_str`.
//!
//! NOTE on the HTTP transport: the server reads the PDF from ITS OWN local filesystem (it is sent
//! only a `pdf_path`, never the bytes). So `marker_url` must point at a server that shares
//! archon's filesystem — same host, or a mount where the identical absolute path resolves. It is
//! NOT a general remote-upload service.

use std::path::{Path, PathBuf};
use std::time::Duration;

use archon_accel::{AccelKind, DeviceOverrides, MarkerChunk, marker_ingest_plan};
use archon_ingest_ext::chunk::{Block, FigureRegion};
use archon_ingest_ext::marker::{parse_marker_figures_str, parse_marker_str};
use archon_policy::PdfPolicy;

use crate::errors::DocsError;

/// Per-document HTTP conversion timeout. Marker on a large scanned book can legitimately take
/// several minutes on GPU (longer on a CPU-fallback), so this is generous — but bounded, so one
/// wedged conversion can't hang an unattended bulk run forever behind the server's convert lock.
/// On timeout the request errors, which the Http transport treats as a hard Marker failure.
pub const HTTP_CONVERT_TIMEOUT_SECS: u64 = 900;

/// Health preflight budget: how long to wait for a just-started Marker server to finish loading
/// its ~6 GB of surya models (it doesn't bind its port until then), and how often to re-poll.
pub const HEALTH_MAX_WAIT_SECS: u64 = 120;
pub const HEALTH_POLL_INTERVAL_SECS: u64 = 2;

/// Where/how to obtain a PDF's Marker block tree.
#[derive(Clone, Debug)]
pub enum MarkerSource {
    /// Spawn the local Python sidecar, once per `archon-accel` chunk. Each chunk runs
    /// `python <script> <pdf> --device <dev> [--page-range S-E]` under its env, walking its own
    /// GPU→CPU OOM ladder; the chunks' block streams concatenate (Marker emits absolute page ids).
    /// A single whole-document chunk (`page_range: None`) is the common case; several page-range
    /// chunks keep a big document on a small card's GPU.
    Subprocess {
        python: String,
        script: PathBuf,
        chunks: Vec<MarkerChunk>,
    },
    /// POST `{"pdf_path", "device"}` to a persistent Marker HTTP server's `/convert` endpoint
    /// (`scripts/archon_marker_server.py`) and get the same normalized block-tree JSON back. The
    /// server loads the surya models ONCE at startup and keeps them resident, so bulk ingest pays
    /// no per-document model reload (the subprocess sidecar reloads ~6 GB per PDF). Server and
    /// archon run on the same host: the absolute `pdf_path` is read locally by the server, so no
    /// bytes are uploaded. `device` is advisory — the server's models live on its startup device.
    Http { url: String, device: Option<String> },
    /// Read a pre-extracted Marker JSON file (decoupled — you run Marker however/whenever).
    PreExtracted { json_path: PathBuf },
}

/// Build a `MarkerSource` from policy. PRECEDENCE: a set `marker_url` → the persistent Marker
/// HTTP server (`MarkerSource::Http`, whole-document, no chunking — the warm server owns its own
/// device/memory). Else a set `marker_sidecar` path → local subprocess with the `archon-accel`
/// GPU→CPU OOM ladder, where the GPU-vs-CPU choice comes from the host's free VRAM vs the
/// **page-scaled** Marker footprint (`page_count`). `marker_device` `None`/`"auto"` →
/// planner-chosen; an explicit `cuda|mps|cpu` forces it. Neither set → `None`.
///
/// NOTE: the subprocess path resolves placement (a cheap `nvidia-smi`/`sysinfo` probe) once per
/// call — in bulk ingest, once per PDF. It can be hoisted to once-per-run if the probe ever shows
/// up in a profile. The Http path skips the probe entirely.
pub fn from_policy(pdf: &PdfPolicy, page_count: u32) -> Option<MarkerSource> {
    if let Some(url) = pdf.marker_url.as_ref() {
        return Some(MarkerSource::Http {
            url: url.clone(),
            device: pdf.marker_device.clone(),
        });
    }
    let script = pdf.marker_sidecar.as_ref()?;
    let chunks = marker_ingest_plan(
        &archon_accel::detect(),
        &overrides_from_policy(pdf),
        page_count,
    );
    Some(MarkerSource::Subprocess {
        python: pdf
            .marker_python
            .clone()
            .unwrap_or_else(|| "python3".to_string()),
        script: PathBuf::from(script),
        chunks,
    })
}

/// Preflight a persistent Marker server's `/health` before an ingest run. The server does not
/// bind its port until its ~6 GB of models finish loading (tens of seconds), so a just-started
/// server is tolerated: poll `{url}/health` every `poll` until it returns
/// `{"status":"ok","models_loaded":true}`, giving up after `max_wait`. Returns `Err` (do NOT
/// ingest) if the server never becomes ready — this is what turns a wrong/forgotten `marker_url`
/// or a still-loading/dead server into a hard stop instead of a silently bbox-less corpus.
pub async fn preflight_health(
    url: &str,
    max_wait: Duration,
    poll: Duration,
) -> Result<(), DocsError> {
    let endpoint = format!("{}/health", url.trim_end_matches('/'));
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()
        .map_err(|e| DocsError::Storage {
            message: format!("marker health client build failed: {e}"),
        })?;
    let deadline = std::time::Instant::now() + max_wait;
    let mut last_err;
    loop {
        match client.get(&endpoint).send().await {
            Ok(resp) => {
                let status = resp.status();
                match resp.text().await {
                    Ok(body) => {
                        if status.is_success() && health_body_ready(&body) {
                            return Ok(());
                        }
                        last_err = format!("status={status} body={}", body.trim());
                    }
                    Err(e) => last_err = format!("status={status} (body read failed: {e})"),
                }
            }
            Err(e) => last_err = e.to_string(),
        }
        if std::time::Instant::now() >= deadline {
            return Err(DocsError::Storage {
                message: format!(
                    "marker server at {url} not ready after {}s (last: {last_err}). \
                     Start it (scripts/archon_marker_server.py) or unset marker_url; \
                     refusing to ingest without the warm Marker server.",
                    max_wait.as_secs()
                ),
            });
        }
        tokio::time::sleep(poll).await;
    }
}

/// True iff a `/health` JSON body reports the server up with models resident.
fn health_body_ready(body: &str) -> bool {
    serde_json::from_str::<serde_json::Value>(body)
        .ok()
        .map(|v| {
            v.get("status").and_then(|s| s.as_str()) == Some("ok")
                && v.get("models_loaded").and_then(|m| m.as_bool()) == Some(true)
        })
        .unwrap_or(false)
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

/// Internal: distinguishes a torch-OOM sidecar failure (advance the ladder) and a signal-kill
/// (e.g. a jetsam/OOM-killer SIGKILL — also advance the ladder; a clean torch OOM never gets to
/// print its signature when the OS kills the process) from any other error.
enum SidecarError {
    Oom {
        device: String,
    },
    Killed {
        device: String,
    },
    /// The per-attempt wall clock elapsed (a wedged sidecar — MPS driver deadlock, surya spin).
    /// Also advances the ladder: a hung GPU convert gets retried on CPU.
    TimedOut {
        device: String,
    },
    Other(DocsError),
}

/// Per-attempt wall-clock bound on ONE marker sidecar conversion. GPU marker is fast, so a long
/// GPU run is a hang → a tight budget catches it and the ladder falls to CPU. CPU marker of a
/// large page-range chunk is legitimately slow → a generous budget avoids false-timeouts (it is
/// the last rung, so an elapse there hard-fails the chunk, exactly as a true hang would).
fn marker_timeout(device: Option<&str>) -> std::time::Duration {
    let (var, default_secs) = if device == Some("cpu") {
        ("ARCHON_MARKER_CPU_TIMEOUT_SECS", 3600u64)
    } else {
        ("ARCHON_MARKER_GPU_TIMEOUT_SECS", 900u64)
    };
    let secs = std::env::var(var)
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(default_secs);
    std::time::Duration::from_secs(secs)
}

impl MarkerSource {
    /// Obtain and parse the Marker block stream for `pdf_path`. For a chunked subprocess plan,
    /// each chunk is run and parsed independently and the block streams are concatenated in page
    /// order (Marker emits absolute page ids, so no re-offset is needed).
    pub async fn blocks_for(&self, pdf_path: &Path) -> Result<Vec<Block>, DocsError> {
        Ok(self.blocks_and_figures_for(pdf_path).await?.0)
    }

    /// Like [`Self::blocks_for`], but ALSO returns the figure/picture regions from the SAME Marker
    /// run (one sidecar invocation per chunk), for the opt-in figure-region VLM path. Figures are
    /// parsed by a separate walk that leaves the text-block stream (and thus chunk parity) untouched.
    pub async fn blocks_and_figures_for(
        &self,
        pdf_path: &Path,
    ) -> Result<(Vec<Block>, Vec<FigureRegion>), DocsError> {
        match self {
            MarkerSource::Subprocess {
                python,
                script,
                chunks,
            } => {
                let mut blocks = Vec::new();
                let mut figures = Vec::new();
                for chunk in chunks {
                    let json = run_chunk(python, script, pdf_path, chunk).await?;
                    blocks.append(&mut parse_blocks(&json)?);
                    figures.append(&mut parse_figures(&json)?);
                }
                Ok((blocks, figures))
            }
            MarkerSource::Http { .. } | MarkerSource::PreExtracted { .. } => {
                let json = self.fetch_json(pdf_path).await?;
                Ok((parse_blocks(&json)?, parse_figures(&json)?))
            }
        }
    }

    /// Fetch whole-document Marker JSON for the non-subprocess transports (persistent HTTP server
    /// or a pre-extracted file — both already carry the full document, so neither chunks).
    ///
    /// HTTP contract (matched by `scripts/archon_marker_server.py`):
    /// `POST {url}/convert` with JSON body `{"pdf_path": "<absolute path>", "device": "<dev>"}` →
    /// 200 with the same normalized block-tree JSON the subprocess sidecar prints to stdout.
    async fn fetch_json(&self, pdf_path: &Path) -> Result<String, DocsError> {
        match self {
            MarkerSource::Http { url, device } => {
                // The server reads the PDF from the local filesystem (same host), so the path
                // must be absolute regardless of archon's cwd.
                let abs =
                    tokio::fs::canonicalize(pdf_path)
                        .await
                        .map_err(|e| DocsError::Storage {
                            message: format!(
                                "canonicalize pdf path for marker http failed ({}): {e}",
                                pdf_path.display()
                            ),
                        })?;
                let body = serde_json::json!({
                    "pdf_path": abs.to_string_lossy(),
                    "device": device.as_deref().unwrap_or("auto"),
                });
                let endpoint = format!("{}/convert", url.trim_end_matches('/'));
                let client = reqwest::Client::builder()
                    .timeout(Duration::from_secs(HTTP_CONVERT_TIMEOUT_SECS))
                    .build()
                    .map_err(|e| DocsError::Storage {
                        message: format!("marker http client build failed: {e}"),
                    })?;
                let resp = client
                    .post(&endpoint)
                    .json(&body)
                    .send()
                    .await
                    .map_err(|e| DocsError::Storage {
                        message: format!("marker http request failed ({endpoint}): {e}"),
                    })?;
                let status = resp.status();
                let text = resp.text().await.map_err(|e| DocsError::Storage {
                    message: format!("marker http response read failed: {e}"),
                })?;
                if !status.is_success() {
                    return Err(DocsError::Storage {
                        message: format!("marker http server returned {status}: {text}"),
                    });
                }
                Ok(text)
            }
            MarkerSource::PreExtracted { json_path } => tokio::fs::read_to_string(json_path)
                .await
                .map_err(|e| DocsError::Storage {
                    message: format!("read pre-extracted marker json failed: {e}"),
                }),
            MarkerSource::Subprocess { .. } => Err(DocsError::Storage {
                message: "internal: subprocess is chunked in blocks_for, not fetch_json"
                    .to_string(),
            }),
        }
    }
}

/// Parse a Marker JSON block tree to `Vec<Block>`, mapping parse failures to a storage error.
fn parse_blocks(json: &str) -> Result<Vec<Block>, DocsError> {
    parse_marker_str(json).map_err(|e| DocsError::Storage {
        message: format!("marker json parse failed: {e}"),
    })
}

/// Parse a Marker JSON block tree to `Vec<FigureRegion>` (the figure-region VLM path).
fn parse_figures(json: &str) -> Result<Vec<FigureRegion>, DocsError> {
    parse_marker_figures_str(json).map_err(|e| DocsError::Storage {
        message: format!("marker figure parse failed: {e}"),
    })
}

/// Run one chunk through its GPU→CPU OOM ladder, passing `--page-range` when the chunk is a
/// page range. Advances to the next rung on a torch-OOM or a signal-kill (an OS memory kill
/// presents as a signal, not exit 42); any other error surfaces.
async fn run_chunk(
    python: &str,
    script: &Path,
    pdf_path: &Path,
    chunk: &MarkerChunk,
) -> Result<String, DocsError> {
    let mut last: Option<DocsError> = None;
    for (device, env) in &chunk.attempts {
        match run_sidecar(
            python,
            script,
            pdf_path,
            Some(device.as_str()),
            env,
            chunk.page_range,
        )
        .await
        {
            Ok(json) => return Ok(json),
            Err(SidecarError::Oom { device: dev }) => {
                last = Some(DocsError::Storage {
                    message: format!("marker OOM on device={dev}"),
                });
            }
            Err(SidecarError::Killed { device: dev }) => {
                last = Some(DocsError::Storage {
                    message: format!("marker killed by signal on device={dev}"),
                });
            }
            Err(SidecarError::TimedOut { device: dev }) => {
                last = Some(DocsError::Storage {
                    message: format!("marker timed out on device={dev}"),
                });
            }
            Err(SidecarError::Other(e)) => return Err(e),
        }
    }
    Err(last.unwrap_or_else(|| DocsError::Storage {
        message: "marker: empty attempt ladder".to_string(),
    }))
}

/// Run the Marker sidecar once with `device` + `env`, classifying a torch-OOM exit (the sidecar
/// exits 42 on OOM, or carries an OOM signature on stderr) and a signal termination (`code()` is
/// `None` — e.g. a jetsam SIGKILL, which leaves no exit code and no stderr signature) so the
/// caller can retry on CPU. The whole conversion is bounded by [`marker_timeout`]'s per-device
/// wall clock; on elapse the child is killed (kill_on_drop) and `TimedOut` advances the ladder.
async fn run_sidecar(
    python: &str,
    script: &Path,
    pdf_path: &Path,
    device: Option<&str>,
    env: &[(String, String)],
    page_range: Option<(u32, u32)>,
) -> Result<String, SidecarError> {
    let mut cmd = tokio::process::Command::new(python);
    cmd.arg(script).arg(pdf_path);
    if let Some(dev) = device {
        cmd.arg("--device").arg(dev);
    }
    if let Some((start, end)) = page_range {
        cmd.arg("--page-range").arg(format!("{start}-{end}"));
    }
    for (k, v) in env {
        cmd.env(k, v);
    }
    // Kill a timed-out child when the output future is dropped by the timeout below — a wedged
    // marker (MPS driver deadlock, surya spin) must not outlive its attempt.
    cmd.kill_on_drop(true);
    let out = match tokio::time::timeout(marker_timeout(device), cmd.output()).await {
        Err(_elapsed) => {
            return Err(SidecarError::TimedOut {
                device: device.unwrap_or("cpu").to_string(),
            });
        }
        Ok(res) => res.map_err(|e| {
            SidecarError::Other(DocsError::Storage {
                message: format!("marker sidecar spawn failed: {e}"),
            })
        })?,
    };
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
    if out.status.code().is_none() {
        // Terminated by a signal (no exit code): treat like OOM and advance the ladder — an OS
        // memory kill (jetsam SIGKILL) looks exactly like this, and a CPU retry beats a hard fail.
        return Err(SidecarError::Killed {
            device: device.unwrap_or("cpu").to_string(),
        });
    }
    Err(SidecarError::Other(DocsError::Storage {
        message: format!("marker sidecar exited {}: {}", out.status, stderr),
    }))
}

#[cfg(test)]
#[path = "marker_source_tests.rs"]
mod tests;
