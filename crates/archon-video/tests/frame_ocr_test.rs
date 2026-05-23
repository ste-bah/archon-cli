use std::path::{Path, PathBuf};

use archon_docs::errors::DocsError;
use archon_docs::ocr::provider::{OcrExtractResult, OcrProvider, OcrRequest};
use archon_video::dedupe::DedupeGroup;
use archon_video::frame_persist::persist_frame_groups;
use archon_video::frames::{ExtractedFrame, compute_frame_hash};
use archon_video::schema::create_video_schema;
use archon_video::store::list_frame_descriptions_for_video;
use archon_video::visual::run_frame_ocr;
use async_trait::async_trait;
use cozo::{DbInstance, ScriptMutability};

struct CwdGuard(PathBuf);

impl Drop for CwdGuard {
    fn drop(&mut self) {
        let _ = std::env::set_current_dir(&self.0);
    }
}

struct MockOcr;

#[async_trait]
impl OcrProvider for MockOcr {
    async fn extract(&self, request: OcrRequest) -> Result<OcrExtractResult, DocsError> {
        assert!(Path::new(&request.file_path).exists());
        assert_eq!(request.page_range, Some((1, 1)));
        Ok(OcrExtractResult {
            full_text: "Revenue rose from Q1 to Q2".into(),
            page_count: 1,
            page_offsets: Vec::new(),
            processing_duration_ms: 1,
        })
    }

    fn name(&self) -> &'static str {
        "mock-ocr"
    }
}

struct FailingOcr;

#[async_trait]
impl OcrProvider for FailingOcr {
    async fn extract(&self, _request: OcrRequest) -> Result<OcrExtractResult, DocsError> {
        Err(DocsError::OcrApi {
            message: "no OCR".into(),
            status_code: None,
        })
    }

    fn name(&self) -> &'static str {
        "failing-ocr"
    }
}

fn setup() -> (tempfile::TempDir, CwdGuard, DbInstance) {
    let dir = tempfile::tempdir().unwrap();
    let previous = std::env::current_dir().unwrap();
    std::env::set_current_dir(dir.path()).unwrap();
    let db = DbInstance::new("mem", "", "").unwrap();
    archon_docs::schema::ensure_doc_schema(&db).unwrap();
    create_video_schema(&db).unwrap();
    (dir, CwdGuard(previous), db)
}

fn persist_test_frame(db: &DbInstance) {
    let frame_dir = Path::new(".archon/video-artifacts/video-1/frames");
    std::fs::create_dir_all(frame_dir).unwrap();
    let image_path = frame_dir.join("frame_0001.jpg");
    image::RgbImage::from_pixel(8, 8, image::Rgb([255, 255, 255]))
        .save(&image_path)
        .unwrap();
    let hash = compute_frame_hash(&image_path).unwrap();
    let group = DedupeGroup {
        dedupe_group_id: "group-1".into(),
        representative: ExtractedFrame {
            timestamp_ms: 1_000,
            timestamp_end_ms: 2_000,
            image_path,
            frame_hash: hash,
            sequence_index: 1,
        },
        representative_hash: 0,
        member_timestamps: vec![1_000],
        first_timestamp_ms: 1_000,
        last_timestamp_ms: 2_000,
        frame_count: 1,
    };
    let mut policy = archon_policy::VideoPolicy::default();
    policy.frames.ocr = true;
    policy.frames.vlm = false;
    persist_frame_groups(
        db,
        "doc-1",
        "video-1",
        "artifact-source",
        &[group],
        &policy,
        "2026-05-22T00:00:00Z",
    )
    .unwrap();
}

#[tokio::test]
async fn frame_ocr_writes_chunks_timeref_and_text() {
    let (_dir, _guard, db) = setup();
    persist_test_frame(&db);
    let frame = list_frame_descriptions_for_video(&db, "video-1")
        .unwrap()
        .remove(0);

    let text = run_frame_ocr(&frame, &db, &MockOcr, "doc-1")
        .await
        .unwrap()
        .unwrap();

    assert_eq!(text, "Revenue rose from Q1 to Q2");
    let updated = list_frame_descriptions_for_video(&db, "video-1")
        .unwrap()
        .remove(0);
    assert_eq!(updated.ocr_text, text);
    let chunks = archon_docs::store::list_chunks_for_doc(&db, "doc-1").unwrap();
    assert_eq!(chunks.len(), 1);
    assert_eq!(chunks[0].content, text);
    let timeref = db
        .run_script(
            "?[start, end] := *video_chunk_timeref{timestamp_start_ms: start, timestamp_end_ms: end}",
            Default::default(),
            ScriptMutability::Immutable,
        )
        .unwrap();
    assert_eq!(timeref.rows.len(), 1);
}

#[tokio::test]
async fn frame_ocr_failure_updates_status_without_chunks() {
    let (_dir, _guard, db) = setup();
    persist_test_frame(&db);
    let frame = list_frame_descriptions_for_video(&db, "video-1")
        .unwrap()
        .remove(0);

    let result = run_frame_ocr(&frame, &db, &FailingOcr, "doc-1")
        .await
        .unwrap();

    assert!(result.is_none());
    let updated = list_frame_descriptions_for_video(&db, "video-1")
        .unwrap()
        .remove(0);
    assert_eq!(updated.status, "ocr_failed");
    assert!(
        archon_docs::store::list_chunks_for_doc(&db, "doc-1")
            .unwrap()
            .is_empty()
    );
}
