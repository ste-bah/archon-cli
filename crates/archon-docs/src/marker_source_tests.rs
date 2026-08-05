use super::*;
use archon_ingest_ext::chunk::BlockType;
use sha2::{Digest, Sha256};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

#[test]
fn marker_http_pdf_id_is_lowercase_sha256_of_exact_canonical_utf8_path() {
    let dir = tempfile::tempdir().unwrap();
    let nested = dir.path().join("nested");
    std::fs::create_dir(&nested).unwrap();
    let pdf = nested.join("résumé.pdf");
    std::fs::write(&pdf, b"%PDF-1.4\n").unwrap();
    let canonical = std::fs::canonicalize(&pdf).unwrap();
    let expected = format!(
        "{:x}",
        Sha256::digest(canonical.to_str().unwrap().as_bytes())
    );

    assert_eq!(pdf_id_for_canonical_path(&canonical).unwrap(), expected);
    assert_eq!(expected.len(), 64);
    assert_eq!(expected, expected.to_lowercase());
}

#[cfg(unix)]
#[test]
fn marker_http_rejects_non_utf8_canonical_path() {
    use std::ffi::OsStr;
    use std::os::unix::ffi::OsStrExt;

    let dir = tempfile::tempdir().unwrap();
    let pdf = dir.path().join(OsStr::from_bytes(b"invalid-\xff.pdf"));
    std::fs::write(&pdf, b"%PDF-1.4\n").unwrap();
    let canonical = std::fs::canonicalize(&pdf).unwrap();

    let err = pdf_id_for_canonical_path(&canonical).unwrap_err();

    assert!(err.to_string().contains("not valid UTF-8"));
}

#[tokio::test]
async fn marker_http_request_sends_pdf_id_without_pdf_path() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/convert"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_string(r#"{"block_type":"Document","children":[]}"#),
        )
        .mount(&server)
        .await;
    let dir = tempfile::tempdir().unwrap();
    let pdf = dir.path().join("report.pdf");
    std::fs::write(&pdf, b"%PDF-1.4\n").unwrap();
    let canonical = std::fs::canonicalize(&pdf).unwrap();
    let expected_id = format!(
        "{:x}",
        Sha256::digest(canonical.to_str().unwrap().as_bytes())
    );
    let source = MarkerSource::Http {
        url: server.uri(),
        device: Some("cpu".to_string()),
    };

    let blocks = source.blocks_for(&pdf).await.unwrap();

    assert!(blocks.is_empty());
    let requests = server.received_requests().await.unwrap();
    assert_eq!(requests.len(), 1);
    let body: serde_json::Value = serde_json::from_slice(&requests[0].body).unwrap();
    assert_eq!(
        body,
        serde_json::json!({
            "pdf_id": expected_id,
            "device": "cpu",
        })
    );
}

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
        chunks: vec![MarkerChunk {
            page_range: None,
            attempts: vec![("cpu".to_string(), vec![])],
        }],
    };
    assert!(src.blocks_for(Path::new("x.pdf")).await.is_err());
}

/// Fake sidecar (run via bash): SIGKILLs itself on any non-cpu device — `status.code()` is
/// `None`, no OOM stderr, exactly the jetsam shape — and emits a valid block tree on cpu.
fn write_signal_kill_sidecar(dir: &Path) -> PathBuf {
    let script = dir.join("fake_sidecar.sh");
    std::fs::write(
        &script,
        r#"#!/usr/bin/env bash
# args: <pdf> --device <dev>
if [ "$3" != "cpu" ]; then kill -9 $$; fi
echo '{"block_type":"Document","children":[{"block_type":"Page","id":"/page/0/Page/0","children":[{"block_type":"Text","html":"<p>cpu ok</p>","bbox":[1,2,3,4]}]}]}'
"#,
    )
    .unwrap();
    script
}

// Runs a sidecar through `bash` and asserts POSIX signal semantics
// (SIGKILL). Windows has neither: `bash` there is either absent or the
// WSL launcher, which would execute in a different filesystem entirely.
#[cfg(unix)]
#[tokio::test]
async fn signal_killed_gpu_attempt_advances_ladder_to_cpu() {
    // A signal-kill on the GPU rung must advance to the CPU rung (like OOM), not hard-fail.
    let dir = tempfile::tempdir().unwrap();
    let src = MarkerSource::Subprocess {
        python: "bash".into(),
        script: write_signal_kill_sidecar(dir.path()),
        chunks: vec![MarkerChunk {
            page_range: None,
            attempts: vec![("mps".to_string(), vec![]), ("cpu".to_string(), vec![])],
        }],
    };
    let blocks = src.blocks_for(Path::new("x.pdf")).await.unwrap();
    assert_eq!(blocks.len(), 1);
    assert_eq!(blocks[0].text, "cpu ok");
}

// Runs a sidecar through `bash` and asserts POSIX signal semantics
// (SIGKILL). Windows has neither: `bash` there is either absent or the
// WSL launcher, which would execute in a different filesystem entirely.
#[cfg(unix)]
#[tokio::test]
async fn signal_kill_on_last_rung_surfaces_killed_error() {
    // With no rung left to advance to, the recorded signal-kill surfaces as the error.
    let dir = tempfile::tempdir().unwrap();
    let src = MarkerSource::Subprocess {
        python: "bash".into(),
        script: write_signal_kill_sidecar(dir.path()),
        chunks: vec![MarkerChunk {
            page_range: None,
            attempts: vec![("mps".to_string(), vec![])],
        }],
    };
    let err = src.blocks_for(Path::new("x.pdf")).await.unwrap_err();
    let msg = format!("{err}");
    assert!(msg.contains("killed by signal"), "got: {msg}");
    assert!(msg.contains("device=mps"), "got: {msg}");
}

/// Fake sidecar (run via bash): HANGS (sleeps well past the test budget) on any non-cpu
/// device — the wedged-GPU shape — and emits a valid block tree on cpu.
fn write_hanging_sidecar(dir: &Path) -> PathBuf {
    let script = dir.join("hanging_sidecar.sh");
    std::fs::write(
        &script,
        r#"#!/usr/bin/env bash
# args: <pdf> --device <dev>
if [ "$3" != "cpu" ]; then sleep 30; fi
echo '{"block_type":"Document","children":[{"block_type":"Page","id":"/page/0/Page/0","children":[{"block_type":"Text","html":"<p>cpu ok</p>","bbox":[1,2,3,4]}]}]}'
"#,
    )
    .unwrap();
    script
}

// Runs a sidecar through `bash`; Windows has no usable one here.
#[cfg(unix)]
#[tokio::test]
#[serial_test::serial(docs_global_state)]
async fn marker_timeout_on_gpu_attempt_advances_ladder_to_cpu() {
    // A hung GPU rung must time out (~1s, NOT the sidecar's 30s sleep) and advance to CPU.
    unsafe {
        std::env::set_var("ARCHON_MARKER_GPU_TIMEOUT_SECS", "1");
    }
    let dir = tempfile::tempdir().unwrap();
    let src = MarkerSource::Subprocess {
        python: "bash".into(),
        script: write_hanging_sidecar(dir.path()),
        chunks: vec![MarkerChunk {
            page_range: None,
            attempts: vec![("mps".to_string(), vec![]), ("cpu".to_string(), vec![])],
        }],
    };
    // Bound the test's own wall clock: without the per-attempt timeout this would sit in the
    // sidecar's 30s sleep, so an unresolved future here IS the regression.
    let blocks = tokio::time::timeout(
        std::time::Duration::from_secs(10),
        src.blocks_for(Path::new("x.pdf")),
    )
    .await
    .expect("ladder must advance past the hung GPU rung within the timeout budget")
    .unwrap();
    unsafe {
        std::env::remove_var("ARCHON_MARKER_GPU_TIMEOUT_SECS");
    }
    assert_eq!(blocks.len(), 1);
    assert_eq!(blocks[0].text, "cpu ok");
}

// Runs a sidecar through `bash`; Windows has no usable one here.
#[cfg(unix)]
#[tokio::test]
#[serial_test::serial(docs_global_state)]
async fn marker_timeout_on_last_rung_surfaces_error() {
    // With no rung left to advance to, the recorded timeout surfaces as the error.
    unsafe {
        std::env::set_var("ARCHON_MARKER_CPU_TIMEOUT_SECS", "1");
    }
    let dir = tempfile::tempdir().unwrap();
    let script = dir.path().join("hanging_cpu_sidecar.sh");
    std::fs::write(&script, "#!/usr/bin/env bash\nsleep 30\n").unwrap();
    let src = MarkerSource::Subprocess {
        python: "bash".into(),
        script,
        chunks: vec![MarkerChunk {
            page_range: None,
            attempts: vec![("cpu".to_string(), vec![])],
        }],
    };
    let err = tokio::time::timeout(
        std::time::Duration::from_secs(10),
        src.blocks_for(Path::new("x.pdf")),
    )
    .await
    .expect("last-rung timeout must resolve to Err, not hang")
    .unwrap_err();
    unsafe {
        std::env::remove_var("ARCHON_MARKER_CPU_TIMEOUT_SECS");
    }
    let msg = format!("{err}");
    assert!(msg.contains("timed out"), "got: {msg}");
    assert!(msg.contains("device=cpu"), "got: {msg}");
}

#[test]
fn from_policy_maps_sidecar_path() {
    let mut pdf = PdfPolicy::default();
    assert!(from_policy(&pdf, 13).is_none(), "no sidecar → None");
    pdf.marker_sidecar = Some("scripts/archon_marker_sidecar.py".into());
    pdf.marker_device = Some("mps".into());
    match from_policy(&pdf, 13) {
        Some(MarkerSource::Subprocess { chunks, .. }) => {
            // Forced mps → a single whole-doc chunk whose preferred attempt is mps.
            assert_eq!(chunks.len(), 1);
            assert_eq!(chunks[0].page_range, None);
            assert_eq!(chunks[0].attempts.first().unwrap().0.as_str(), "mps");
        }
        other => panic!("expected subprocess, got {other:?}"),
    }
}

#[test]
fn from_policy_marker_url_takes_precedence_over_sidecar() {
    // marker_url alone → Http (warm server).
    let mut pdf = PdfPolicy {
        marker_url: Some("http://127.0.0.1:8010".into()),
        marker_device: Some("cuda".into()),
        ..Default::default()
    };
    match from_policy(&pdf, 13) {
        Some(MarkerSource::Http { url, device }) => {
            assert_eq!(url, "http://127.0.0.1:8010");
            assert_eq!(device.as_deref(), Some("cuda"));
        }
        other => panic!("expected http, got {other:?}"),
    }
    // marker_url + marker_sidecar → still Http (url wins).
    pdf.marker_sidecar = Some("scripts/archon_marker_sidecar.py".into());
    assert!(matches!(
        from_policy(&pdf, 13),
        Some(MarkerSource::Http { .. })
    ));
    // Only marker_sidecar → Subprocess (today's behavior, unchanged).
    pdf.marker_url = None;
    assert!(matches!(
        from_policy(&pdf, 13),
        Some(MarkerSource::Subprocess { .. })
    ));
}

#[test]
fn health_body_ready_requires_ok_and_models_loaded() {
    assert!(health_body_ready(
        r#"{"status":"ok","device":"cuda","models_loaded":true}"#
    ));
    assert!(!health_body_ready(
        r#"{"status":"ok","models_loaded":false}"#
    ));
    assert!(!health_body_ready(
        r#"{"status":"loading","models_loaded":true}"#
    ));
    assert!(!health_body_ready("not json"));
    assert!(!health_body_ready(r#"{"status":"ok"}"#));
}

#[tokio::test]
async fn preflight_fails_fast_when_server_down() {
    // Port 9 (discard) refuses connections; the poll loop must give up at the deadline and
    // return Err rather than proceeding. Short budget so the test is fast.
    let res = preflight_health(
        "http://127.0.0.1:9",
        Duration::from_millis(400),
        Duration::from_millis(100),
    )
    .await;
    assert!(res.is_err(), "a dead server must fail preflight, not pass");
    let msg = format!("{}", res.unwrap_err());
    assert!(msg.contains("not ready"), "clear message; got: {msg}");
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
    match from_policy(&p, 13) {
        Some(MarkerSource::Subprocess { chunks, .. }) => {
            let (device, env) = chunks.first().unwrap().attempts.first().unwrap();
            assert_eq!(device.as_str(), "cpu");
            assert!(env.iter().any(|(k, _)| k == "TORCH_DEVICE"));
        }
        other => panic!("expected subprocess, got {other:?}"),
    }
}
