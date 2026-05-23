use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use archon_docs::errors::DocsError;
use archon_docs::vlm::{VIDEO_FRAME_PROMPT, VlmDescriptionProvider};
use archon_video::dedupe::DedupeGroup;
use archon_video::frame_persist::persist_frame_groups;
use archon_video::frames::{ExtractedFrame, compute_frame_hash};
use archon_video::schema::create_video_schema;
use archon_video::store::list_frame_descriptions_for_video;
use archon_video::visual::run_frame_vlm;
use cozo::DbInstance;

struct CwdGuard(PathBuf);

impl Drop for CwdGuard {
    fn drop(&mut self) {
        let _ = std::env::set_current_dir(&self.0);
        archon_docs::vlm::clear_provider();
    }
}

struct PromptRecordingVlm {
    prompts: Arc<Mutex<Vec<Option<String>>>>,
}

impl VlmDescriptionProvider for PromptRecordingVlm {
    fn describe_image(
        &self,
        _image_bytes: &[u8],
        prompt: Option<&str>,
    ) -> Result<String, DocsError> {
        self.prompts
            .lock()
            .unwrap()
            .push(prompt.map(|prompt| prompt.to_string()));
        Ok("Slide shows a configurable scorecard decision flow".into())
    }
}

struct FailingVlm;

impl VlmDescriptionProvider for FailingVlm {
    fn describe_image(
        &self,
        _image_bytes: &[u8],
        _prompt: Option<&str>,
    ) -> Result<String, DocsError> {
        Err(DocsError::VlmProvider {
            provider: "mock".into(),
            message: "vlm down".into(),
            status_code: None,
        })
    }
}

fn setup() -> (tempfile::TempDir, CwdGuard, DbInstance) {
    archon_docs::vlm::clear_provider();
    let dir = tempfile::tempdir().unwrap();
    let previous = std::env::current_dir().unwrap();
    std::env::set_current_dir(dir.path()).unwrap();
    let db = DbInstance::new("mem", "", "").unwrap();
    archon_docs::schema::ensure_doc_schema(&db).unwrap();
    create_video_schema(&db).unwrap();
    (dir, CwdGuard(previous), db)
}

fn policy() -> archon_policy::EffectivePolicy {
    let mut policy = archon_policy::EffectivePolicy::default();
    policy.video.enabled = true;
    policy.video.frames.vlm = true;
    policy.docs.vlm.enabled = true;
    policy.docs.vlm.mode = "local".into();
    policy.docs.vlm.provider = "ollama".into();
    policy.workers.vlm = "allow-local".into();
    policy
}

fn persist_test_frame(db: &DbInstance) {
    let frame_dir = Path::new(".archon/video-artifacts/video-1/frames");
    std::fs::create_dir_all(frame_dir).unwrap();
    let image_path = frame_dir.join("frame_0001.jpg");
    image::RgbImage::from_pixel(8, 8, image::Rgb([0, 0, 255]))
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
    let mut video_policy = archon_policy::VideoPolicy::default();
    video_policy.frames.ocr = false;
    video_policy.frames.vlm = true;
    persist_frame_groups(
        db,
        "doc-1",
        "video-1",
        "artifact-source",
        &[group],
        &video_policy,
        "2026-05-22T00:00:00Z",
    )
    .unwrap();
}

#[tokio::test]
async fn frame_vlm_uses_video_prompt_and_writes_chunk() {
    let (_dir, _guard, db) = setup();
    persist_test_frame(&db);
    let prompts = Arc::new(Mutex::new(Vec::new()));
    archon_docs::vlm::set_provider(Box::new(PromptRecordingVlm {
        prompts: Arc::clone(&prompts),
    }));
    let frame = list_frame_descriptions_for_video(&db, "video-1")
        .unwrap()
        .remove(0);

    let text = run_frame_vlm(&frame, &db, &policy(), "doc-1")
        .await
        .unwrap()
        .unwrap();

    assert!(text.contains("scorecard"));
    assert_eq!(
        prompts.lock().unwrap()[0].as_deref(),
        Some(VIDEO_FRAME_PROMPT)
    );
    let updated = list_frame_descriptions_for_video(&db, "video-1")
        .unwrap()
        .remove(0);
    assert_eq!(updated.vlm_description, text);
    let chunks = archon_docs::store::list_chunks_for_doc(&db, "doc-1").unwrap();
    assert_eq!(chunks.len(), 1);
    assert_eq!(chunks[0].content, text);
}

#[tokio::test]
async fn frame_vlm_failure_marks_frame_and_keeps_chunks_empty() {
    let (_dir, _guard, db) = setup();
    persist_test_frame(&db);
    archon_docs::vlm::set_provider(Box::new(FailingVlm));
    let frame = list_frame_descriptions_for_video(&db, "video-1")
        .unwrap()
        .remove(0);

    let result = run_frame_vlm(&frame, &db, &policy(), "doc-1")
        .await
        .unwrap();

    assert!(result.is_none());
    let updated = list_frame_descriptions_for_video(&db, "video-1")
        .unwrap()
        .remove(0);
    assert_eq!(updated.status, "vlm_failed");
    assert!(
        archon_docs::store::list_chunks_for_doc(&db, "doc-1")
            .unwrap()
            .is_empty()
    );
}
