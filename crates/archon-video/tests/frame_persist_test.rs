use cozo::DbInstance;

use archon_video::dedupe::DedupeGroup;
use archon_video::frame_persist::persist_frame_groups;
use archon_video::frames::ExtractedFrame;
use archon_video::schema::create_video_schema;
use archon_video::store::list_frame_descriptions_for_video;

fn group() -> DedupeGroup {
    DedupeGroup {
        dedupe_group_id: "dedupe-1".into(),
        representative: ExtractedFrame {
            timestamp_ms: 1_000,
            timestamp_end_ms: 2_000,
            image_path: "frame.jpg".into(),
            frame_hash: "a".repeat(64),
            sequence_index: 1,
        },
        representative_hash: 42,
        member_timestamps: vec![1_000, 1_500, 2_000],
        first_timestamp_ms: 1_000,
        last_timestamp_ms: 2_000,
        frame_count: 3,
    }
}

#[test]
fn frame_groups_create_descriptions_artifacts_and_provenance() {
    let db = DbInstance::new("mem", "", "").unwrap();
    archon_docs::schema::ensure_doc_schema(&db).unwrap();
    create_video_schema(&db).unwrap();
    let mut policy = archon_policy::VideoPolicy::default();
    policy.frames.ocr = true;
    policy.frames.vlm = false;

    let count = persist_frame_groups(
        &db,
        "doc-1",
        "video-1",
        "artifact-source",
        &[group()],
        &policy,
        "2026-05-22T00:00:00Z",
    )
    .unwrap();

    assert_eq!(count, 1);
    let frames = list_frame_descriptions_for_video(&db, "video-1").unwrap();
    assert_eq!(frames.len(), 1);
    assert_eq!(frames[0].timestamp_ms, 1_000);
    assert_eq!(frames[0].timestamp_end_ms, 2_000);
    assert_eq!(frames[0].dedupe_group_id, "dedupe-1");
    assert_eq!(frames[0].status, "pending_ocr_vlm");

    let artifacts = archon_docs::store::list_artifacts_for_doc(&db, "doc-1").unwrap();
    assert_eq!(artifacts.len(), 1);
    assert_eq!(artifacts[0].artifact_type, "video_frame_ocr");
    let edges =
        archon_docs::store::list_provenance_from(&db, &frames[0].image_artifact_id).unwrap();
    assert_eq!(edges.len(), 1);
    assert_eq!(edges[0].to_artifact_id, "artifact-source");
}
