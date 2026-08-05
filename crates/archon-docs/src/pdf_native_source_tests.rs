use super::*;
use archon_ingest_ext::chunk::BlockType;

fn policy_with(script: Option<String>, enabled: bool) -> PdfPolicy {
    PdfPolicy {
        use_pdf_native_extractor: enabled,
        pdf_native_script: script,
        ..PdfPolicy::default()
    }
}

#[test]
fn from_policy_none_when_disabled() {
    // Even with a valid script configured, the kill switch wins.
    let script = repo_script();
    let p = policy_with(Some(script.to_string_lossy().into_owned()), false);
    assert!(PdfNativeSource::from_policy(&p).is_none());
}

#[test]
fn from_policy_none_when_script_missing() {
    let p = policy_with(Some("/nonexistent/sidecar.py".into()), true);
    assert!(
        PdfNativeSource::from_policy(&p).is_none(),
        "a configured-but-absent script must disable the source, not error later"
    );
}

/// The checked-in sidecar, located relative to this crate (works under `cargo test`
/// from any workspace member).
fn repo_script() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../scripts/archon_pdf_native_sidecar.py")
}

#[tokio::test]
async fn sidecar_failure_surfaces_as_error() {
    let src = PdfNativeSource {
        python: "python3".into(),
        script: PathBuf::from("/nonexistent/archon_pdf_native_sidecar.py"),
    };
    assert!(src.blocks_for(Path::new("x.pdf")).await.is_err());
}

#[tokio::test]
async fn sidecar_nonzero_exit_surfaces_as_error() {
    // A real sidecar pointed at a missing PDF exits 3 → Err (caller falls back).
    let script = repo_script();
    assert!(script.is_file(), "sidecar script missing from scripts/");
    let src = PdfNativeSource {
        python: "python3".into(),
        script,
    };
    let err = src
        .blocks_for(Path::new("/nonexistent/document.pdf"))
        .await
        .expect_err("missing pdf must error");
    let msg = format!("{err}");
    assert!(msg.contains("pdf-native"), "got: {msg}");
}

#[tokio::test]
async fn fake_sidecar_output_parses_to_blocks() {
    // Contract test without a PDF: a stand-in "sidecar" that prints a Marker-format
    // tree exercises the spawn → stdout → parse_marker_str pipeline end to end.
    let dir = tempfile::tempdir().unwrap();
    let script = dir.path().join("fake_native.py");
    std::fs::write(
        &script,
        r#"import json, sys
tree = {"block_type": "Document", "children": [{
    "block_type": "Page", "id": "/page/0/Page/0", "bbox": [0, 0, 612, 792],
    "children": [
        {"block_type": "SectionHeader", "id": "/page/0/SectionHeader/0",
         "html": "<h2>Intro</h2>", "bbox": [70.65, 87.39, 272.83, 106.14], "children": []},
        {"block_type": "Text", "id": "/page/0/Text/1",
         "html": "<p>A native-coordinate paragraph.</p>",
         "bbox": [70.65, 113.47, 496.58, 215.62], "children": []}
    ]}]}
print(json.dumps(tree))
"#,
    )
    .unwrap();
    let src = PdfNativeSource {
        python: "python3".into(),
        script,
    };
    let blocks = src
        .blocks_for(Path::new("ignored.pdf"))
        .await
        .unwrap()
        .expect("blocks present");
    assert_eq!(blocks.len(), 2);
    assert_eq!(blocks[0].block_type, BlockType::SectionHeader);
    assert_eq!(blocks[0].text, "Intro");
    assert_eq!(blocks[0].page, 1, "/page/0/ → 1-indexed page 1");
    assert_eq!(blocks[1].bbox, [70.65, 113.47, 496.58, 215.62]);
}

#[tokio::test]
async fn empty_tree_yields_none() {
    // An image-only PDF produces pages but no leaf blocks → Ok(None), caller falls back.
    let dir = tempfile::tempdir().unwrap();
    let script = dir.path().join("empty_native.py");
    std::fs::write(
        &script,
        r#"print('{"block_type": "Document", "children": []}')"#,
    )
    .unwrap();
    let src = PdfNativeSource {
        python: "python3".into(),
        script,
    };
    assert!(
        src.blocks_for(Path::new("ignored.pdf"))
            .await
            .unwrap()
            .is_none()
    );
}

#[tokio::test]
async fn real_sidecar_selftest_fixture_roundtrip() {
    // Gate (plan Phase 3): the REAL sidecar's --selftest output parses to ≥ 1 block with a
    // non-zero bbox. Runs the actual python script; skips silently only if python3 itself
    // is absent (CI images always have it).
    let script = repo_script();
    assert!(script.is_file(), "sidecar script missing from scripts/");
    let out = tokio::process::Command::new("python3")
        .arg(&script)
        .arg("--selftest")
        .output()
        .await
        .expect("python3 runs");
    assert!(
        out.status.success(),
        "selftest failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let blocks = parse_marker_str(&String::from_utf8(out.stdout).unwrap()).unwrap();
    assert!(!blocks.is_empty(), "selftest emits blocks");
    assert!(
        blocks
            .iter()
            .all(|b| b.bbox[2] > b.bbox[0] && b.bbox[3] > b.bbox[1]),
        "all selftest bboxes non-degenerate"
    );
    // The selftest's known landmarks survive the full pipeline into Rust blocks.
    assert!(
        blocks
            .iter()
            .any(|b| b.block_type == BlockType::SectionHeader
                && b.text.contains("Native Extraction"))
    );
    assert!(
        blocks
            .iter()
            .any(|b| b.text.contains("analysis of position")),
        "de-hyphenation visible from Rust"
    );
    assert!(
        blocks.iter().any(|b| b.text.starts_with("[TABLE]")),
        "grid detected and gated into a [TABLE] chunk by is_real_table"
    );
}
