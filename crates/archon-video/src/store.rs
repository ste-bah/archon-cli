use std::collections::BTreeMap;

use anyhow::Result;
use cozo::{DataValue, DbInstance, ScriptMutability};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VideoSource {
    pub video_id: String,
    pub document_id: String,
    pub source_kind: String,
    pub source_url: String,
    pub local_path: String,
    pub title: String,
    pub channel_or_author: String,
    pub duration_ms: i64,
    pub published_at: String,
    pub license: String,
    pub source_hash: String,
    pub ingest_status: String,
    pub policy_snapshot_json: String,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VideoTrack {
    pub track_id: String,
    pub video_id: String,
    pub track_kind: String,
    pub provider: String,
    pub model: String,
    pub status: String,
    pub warning_count: i64,
    pub error_count: i64,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TranscriptSegment {
    pub segment_id: String,
    pub video_id: String,
    pub track_id: String,
    pub start_ms: i64,
    pub end_ms: i64,
    pub speaker: String,
    pub text: String,
    pub confidence: f64,
    pub source_method: String,
    pub chunk_id: String,
    pub created_at: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FrameDescription {
    pub frame_id: String,
    pub video_id: String,
    pub track_id: String,
    pub timestamp_ms: i64,
    pub timestamp_end_ms: i64,
    pub frame_hash: String,
    pub perceptual_hash: String,
    pub image_artifact_id: String,
    pub ocr_text: String,
    pub vlm_description: String,
    pub provider: String,
    pub model: String,
    pub cost_usd: f64,
    pub chunk_id: String,
    pub dedupe_group_id: String,
    pub status: String,
    pub warning: String,
    pub created_at: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ChunkTimeRef {
    pub chunk_id: String,
    pub video_id: String,
    pub track_id: String,
    pub timestamp_start_ms: i64,
    pub timestamp_end_ms: i64,
    pub created_at: String,
}

pub fn insert_video_source(db: &DbInstance, source: &VideoSource) -> Result<()> {
    let mut p = params();
    put_strs(
        &mut p,
        &[
            ("vid", &source.video_id),
            ("did", &source.document_id),
            ("kind", &source.source_kind),
            ("url", &source.source_url),
            ("path", &source.local_path),
            ("title", &source.title),
            ("author", &source.channel_or_author),
            ("pub", &source.published_at),
            ("lic", &source.license),
            ("hash", &source.source_hash),
            ("status", &source.ingest_status),
            ("policy", &source.policy_snapshot_json),
            ("cat", &source.created_at),
            ("uat", &source.updated_at),
        ],
    );
    p.insert("dur".into(), DataValue::from(source.duration_ms));
    db.run_script(
        "?[video_id, document_id, source_kind, source_url, local_path, title, channel_or_author, duration_ms, published_at, license, source_hash, ingest_status, policy_snapshot_json, created_at, updated_at] <- [[$vid, $did, $kind, $url, $path, $title, $author, $dur, $pub, $lic, $hash, $status, $policy, $cat, $uat]]
         :put video_sources { video_id => document_id, source_kind, source_url, local_path, title, channel_or_author, duration_ms, published_at, license, source_hash, ingest_status, policy_snapshot_json, created_at, updated_at }",
        p,
        ScriptMutability::Mutable,
    )
    .map_err(|e| anyhow::anyhow!("insert video_sources failed: {e}"))?;
    Ok(())
}

pub fn get_video_source(db: &DbInstance, video_id: &str) -> Result<Option<VideoSource>> {
    let mut p = params();
    p.insert("vid".into(), DataValue::from(video_id));
    let result = db.run_script(
        "?[video_id, document_id, source_kind, source_url, local_path, title, channel_or_author, duration_ms, published_at, license, source_hash, ingest_status, policy_snapshot_json, created_at, updated_at] := *video_sources{video_id, document_id, source_kind, source_url, local_path, title, channel_or_author, duration_ms, published_at, license, source_hash, ingest_status, policy_snapshot_json, created_at, updated_at}, video_id = $vid",
        p,
        ScriptMutability::Immutable,
    )
    .map_err(|e| anyhow::anyhow!("get video_sources failed: {e}"))?;
    Ok(result.rows.first().map(|row| row_to_video_source(row)))
}

pub fn get_video_by_hash(db: &DbInstance, source_hash: &str) -> Result<Option<VideoSource>> {
    let mut p = params();
    p.insert("hash".into(), DataValue::from(source_hash));
    let result = db.run_script(
        "?[video_id, document_id, source_kind, source_url, local_path, title, channel_or_author, duration_ms, published_at, license, source_hash, ingest_status, policy_snapshot_json, created_at, updated_at] := *video_sources{video_id, document_id, source_kind, source_url, local_path, title, channel_or_author, duration_ms, published_at, license, source_hash, ingest_status, policy_snapshot_json, created_at, updated_at}, source_hash = $hash",
        p,
        ScriptMutability::Immutable,
    )
    .map_err(|e| anyhow::anyhow!("get video_sources by hash failed: {e}"))?;
    Ok(result.rows.first().map(|row| row_to_video_source(row)))
}

pub fn list_video_sources(db: &DbInstance) -> Result<Vec<VideoSource>> {
    let result = db.run_script(
        "?[video_id, document_id, source_kind, source_url, local_path, title, channel_or_author, duration_ms, published_at, license, source_hash, ingest_status, policy_snapshot_json, created_at, updated_at] := *video_sources{video_id, document_id, source_kind, source_url, local_path, title, channel_or_author, duration_ms, published_at, license, source_hash, ingest_status, policy_snapshot_json, created_at, updated_at}",
        Default::default(),
        ScriptMutability::Immutable,
    )
    .map_err(|e| anyhow::anyhow!("list video_sources failed: {e}"))?;
    Ok(result
        .rows
        .iter()
        .map(|row| row_to_video_source(row))
        .collect())
}

pub fn update_video_status(
    db: &DbInstance,
    video_id: &str,
    status: &str,
    updated_at: &str,
) -> Result<()> {
    let mut source = get_video_source(db, video_id)?
        .ok_or_else(|| anyhow::anyhow!("video source not found: {video_id}"))?;
    source.ingest_status = status.to_string();
    source.updated_at = updated_at.to_string();
    insert_video_source(db, &source)
}

pub fn insert_video_track(db: &DbInstance, track: &VideoTrack) -> Result<()> {
    let mut p = params();
    put_strs(
        &mut p,
        &[
            ("tid", &track.track_id),
            ("vid", &track.video_id),
            ("kind", &track.track_kind),
            ("prov", &track.provider),
            ("model", &track.model),
            ("status", &track.status),
            ("cat", &track.created_at),
            ("uat", &track.updated_at),
        ],
    );
    p.insert("warn".into(), DataValue::from(track.warning_count));
    p.insert("err".into(), DataValue::from(track.error_count));
    db.run_script(
        "?[track_id, video_id, track_kind, provider, model, status, warning_count, error_count, created_at, updated_at] <- [[$tid, $vid, $kind, $prov, $model, $status, $warn, $err, $cat, $uat]]
         :put video_tracks { track_id => video_id, track_kind, provider, model, status, warning_count, error_count, created_at, updated_at }",
        p,
        ScriptMutability::Mutable,
    )
    .map_err(|e| anyhow::anyhow!("insert video_tracks failed: {e}"))?;
    Ok(())
}

pub fn get_video_track(db: &DbInstance, track_id: &str) -> Result<Option<VideoTrack>> {
    let mut p = params();
    p.insert("tid".into(), DataValue::from(track_id));
    let result = db.run_script(
        "?[track_id, video_id, track_kind, provider, model, status, warning_count, error_count, created_at, updated_at] := *video_tracks{track_id, video_id, track_kind, provider, model, status, warning_count, error_count, created_at, updated_at}, track_id = $tid",
        p,
        ScriptMutability::Immutable,
    )
    .map_err(|e| anyhow::anyhow!("get video_tracks failed: {e}"))?;
    Ok(result.rows.first().map(|row| VideoTrack {
        track_id: row[0].get_str().unwrap_or("").to_string(),
        video_id: row[1].get_str().unwrap_or("").to_string(),
        track_kind: row[2].get_str().unwrap_or("").to_string(),
        provider: row[3].get_str().unwrap_or("").to_string(),
        model: row[4].get_str().unwrap_or("").to_string(),
        status: row[5].get_str().unwrap_or("").to_string(),
        warning_count: row[6].get_int().unwrap_or(0),
        error_count: row[7].get_int().unwrap_or(0),
        created_at: row[8].get_str().unwrap_or("").to_string(),
        updated_at: row[9].get_str().unwrap_or("").to_string(),
    }))
}

pub fn update_track_status(
    db: &DbInstance,
    track_id: &str,
    status: &str,
    warning_count: i64,
    error_count: i64,
    updated_at: &str,
) -> Result<()> {
    let mut track = get_video_track(db, track_id)?
        .ok_or_else(|| anyhow::anyhow!("video track not found: {track_id}"))?;
    track.status = status.to_string();
    track.warning_count = warning_count;
    track.error_count = error_count;
    track.updated_at = updated_at.to_string();
    insert_video_track(db, &track)
}

pub fn insert_transcript_segment(db: &DbInstance, segment: &TranscriptSegment) -> Result<()> {
    let mut p = params();
    put_strs(
        &mut p,
        &[
            ("sid", &segment.segment_id),
            ("vid", &segment.video_id),
            ("tid", &segment.track_id),
            ("speaker", &segment.speaker),
            ("text", &segment.text),
            ("method", &segment.source_method),
            ("cid", &segment.chunk_id),
            ("cat", &segment.created_at),
        ],
    );
    p.insert("start".into(), DataValue::from(segment.start_ms));
    p.insert("end".into(), DataValue::from(segment.end_ms));
    p.insert("conf".into(), DataValue::from(segment.confidence));
    db.run_script(
        "?[segment_id, video_id, track_id, start_ms, end_ms, speaker, text, confidence, source_method, chunk_id, created_at] <- [[$sid, $vid, $tid, $start, $end, $speaker, $text, $conf, $method, $cid, $cat]]
         :put video_transcript_segments { segment_id => video_id, track_id, start_ms, end_ms, speaker, text, confidence, source_method, chunk_id, created_at }",
        p,
        ScriptMutability::Mutable,
    )
    .map_err(|e| anyhow::anyhow!("insert video_transcript_segments failed: {e}"))?;
    Ok(())
}

pub fn get_transcript_segments_for_video(
    db: &DbInstance,
    video_id: &str,
) -> Result<Vec<TranscriptSegment>> {
    let mut p = params();
    p.insert("vid".into(), DataValue::from(video_id));
    let result = db.run_script(
        "?[segment_id, video_id, track_id, start_ms, end_ms, speaker, text, confidence, source_method, chunk_id, created_at] := *video_transcript_segments{segment_id, video_id, track_id, start_ms, end_ms, speaker, text, confidence, source_method, chunk_id, created_at}, video_id = $vid",
        p,
        ScriptMutability::Immutable,
    )
    .map_err(|e| anyhow::anyhow!("get video_transcript_segments failed: {e}"))?;
    let mut rows: Vec<_> = result
        .rows
        .iter()
        .map(|row| row_to_transcript_segment(row))
        .collect();
    rows.sort_by_key(|segment| segment.start_ms);
    Ok(rows)
}

pub fn insert_frame_description(db: &DbInstance, frame: &FrameDescription) -> Result<()> {
    let mut p = params();
    put_strs(
        &mut p,
        &[
            ("fid", &frame.frame_id),
            ("vid", &frame.video_id),
            ("tid", &frame.track_id),
            ("hash", &frame.frame_hash),
            ("phash", &frame.perceptual_hash),
            ("artifact", &frame.image_artifact_id),
            ("ocr", &frame.ocr_text),
            ("vlm", &frame.vlm_description),
            ("prov", &frame.provider),
            ("model", &frame.model),
            ("cid", &frame.chunk_id),
            ("dgid", &frame.dedupe_group_id),
            ("status", &frame.status),
            ("warning", &frame.warning),
            ("cat", &frame.created_at),
        ],
    );
    p.insert("ts".into(), DataValue::from(frame.timestamp_ms));
    p.insert("te".into(), DataValue::from(frame.timestamp_end_ms));
    p.insert("cost".into(), DataValue::from(frame.cost_usd));
    db.run_script(
        "?[frame_id, video_id, track_id, timestamp_ms, timestamp_end_ms, frame_hash, perceptual_hash, image_artifact_id, ocr_text, vlm_description, provider, model, cost_usd, chunk_id, dedupe_group_id, status, warning, created_at] <- [[$fid, $vid, $tid, $ts, $te, $hash, $phash, $artifact, $ocr, $vlm, $prov, $model, $cost, $cid, $dgid, $status, $warning, $cat]]
         :put video_frame_descriptions { frame_id => video_id, track_id, timestamp_ms, timestamp_end_ms, frame_hash, perceptual_hash, image_artifact_id, ocr_text, vlm_description, provider, model, cost_usd, chunk_id, dedupe_group_id, status, warning, created_at }",
        p,
        ScriptMutability::Mutable,
    )
    .map_err(|e| anyhow::anyhow!("insert video_frame_descriptions failed: {e}"))?;
    Ok(())
}

pub fn list_frame_descriptions_for_video(
    db: &DbInstance,
    video_id: &str,
) -> Result<Vec<FrameDescription>> {
    let mut p = params();
    p.insert("vid".into(), DataValue::from(video_id));
    let result = db.run_script(
        "?[frame_id, video_id, track_id, timestamp_ms, timestamp_end_ms, frame_hash, perceptual_hash, image_artifact_id, ocr_text, vlm_description, provider, model, cost_usd, chunk_id, dedupe_group_id, status, warning, created_at] := *video_frame_descriptions{frame_id, video_id, track_id, timestamp_ms, timestamp_end_ms, frame_hash, perceptual_hash, image_artifact_id, ocr_text, vlm_description, provider, model, cost_usd, chunk_id, dedupe_group_id, status, warning, created_at}, video_id = $vid",
        p,
        ScriptMutability::Immutable,
    )
    .map_err(|e| anyhow::anyhow!("list video_frame_descriptions failed: {e}"))?;
    Ok(result
        .rows
        .iter()
        .map(|row| row_to_frame_description(row))
        .collect())
}

pub fn get_frame_description(db: &DbInstance, frame_id: &str) -> Result<Option<FrameDescription>> {
    let mut p = params();
    p.insert("fid".into(), DataValue::from(frame_id));
    let result = db.run_script(
        "?[frame_id, video_id, track_id, timestamp_ms, timestamp_end_ms, frame_hash, perceptual_hash, image_artifact_id, ocr_text, vlm_description, provider, model, cost_usd, chunk_id, dedupe_group_id, status, warning, created_at] := *video_frame_descriptions{frame_id, video_id, track_id, timestamp_ms, timestamp_end_ms, frame_hash, perceptual_hash, image_artifact_id, ocr_text, vlm_description, provider, model, cost_usd, chunk_id, dedupe_group_id, status, warning, created_at}, frame_id = $fid",
        p,
        ScriptMutability::Immutable,
    )
    .map_err(|e| anyhow::anyhow!("get video_frame_descriptions failed: {e}"))?;
    Ok(result.rows.first().map(|row| row_to_frame_description(row)))
}

pub fn update_frame_ocr_text(db: &DbInstance, frame_id: &str, text: &str) -> Result<()> {
    let mut frame = get_frame_description(db, frame_id)?
        .ok_or_else(|| anyhow::anyhow!("frame description not found: {frame_id}"))?;
    frame.ocr_text = text.to_string();
    insert_frame_description(db, &frame)
}

pub fn update_frame_vlm_description(db: &DbInstance, frame_id: &str, text: &str) -> Result<()> {
    let mut frame = get_frame_description(db, frame_id)?
        .ok_or_else(|| anyhow::anyhow!("frame description not found: {frame_id}"))?;
    frame.vlm_description = text.to_string();
    insert_frame_description(db, &frame)
}

pub fn update_frame_status(
    db: &DbInstance,
    frame_id: &str,
    status: &str,
    warning: &str,
) -> Result<()> {
    let mut frame = get_frame_description(db, frame_id)?
        .ok_or_else(|| anyhow::anyhow!("frame description not found: {frame_id}"))?;
    frame.status = status.to_string();
    frame.warning = warning.to_string();
    insert_frame_description(db, &frame)
}

pub fn insert_chunk_timeref(db: &DbInstance, timeref: &ChunkTimeRef) -> Result<()> {
    let mut p = params();
    put_strs(
        &mut p,
        &[
            ("cid", &timeref.chunk_id),
            ("vid", &timeref.video_id),
            ("tid", &timeref.track_id),
            ("cat", &timeref.created_at),
        ],
    );
    p.insert("start".into(), DataValue::from(timeref.timestamp_start_ms));
    p.insert("end".into(), DataValue::from(timeref.timestamp_end_ms));
    db.run_script(
        "?[chunk_id, video_id, track_id, timestamp_start_ms, timestamp_end_ms, created_at] <- [[$cid, $vid, $tid, $start, $end, $cat]]
         :put video_chunk_timeref { chunk_id => video_id, track_id, timestamp_start_ms, timestamp_end_ms, created_at }",
        p,
        ScriptMutability::Mutable,
    )
    .map_err(|e| anyhow::anyhow!("insert video_chunk_timeref failed: {e}"))?;
    Ok(())
}

fn params() -> BTreeMap<String, DataValue> {
    BTreeMap::new()
}

fn put_strs(params: &mut BTreeMap<String, DataValue>, values: &[(&str, &str)]) {
    for (key, value) in values {
        params.insert((*key).to_string(), DataValue::from(*value));
    }
}

fn row_to_video_source(row: &[DataValue]) -> VideoSource {
    VideoSource {
        video_id: row[0].get_str().unwrap_or("").to_string(),
        document_id: row[1].get_str().unwrap_or("").to_string(),
        source_kind: row[2].get_str().unwrap_or("").to_string(),
        source_url: row[3].get_str().unwrap_or("").to_string(),
        local_path: row[4].get_str().unwrap_or("").to_string(),
        title: row[5].get_str().unwrap_or("").to_string(),
        channel_or_author: row[6].get_str().unwrap_or("").to_string(),
        duration_ms: row[7].get_int().unwrap_or(0),
        published_at: row[8].get_str().unwrap_or("").to_string(),
        license: row[9].get_str().unwrap_or("").to_string(),
        source_hash: row[10].get_str().unwrap_or("").to_string(),
        ingest_status: row[11].get_str().unwrap_or("").to_string(),
        policy_snapshot_json: row[12].get_str().unwrap_or("").to_string(),
        created_at: row[13].get_str().unwrap_or("").to_string(),
        updated_at: row[14].get_str().unwrap_or("").to_string(),
    }
}

fn row_to_frame_description(row: &[DataValue]) -> FrameDescription {
    FrameDescription {
        frame_id: row[0].get_str().unwrap_or("").to_string(),
        video_id: row[1].get_str().unwrap_or("").to_string(),
        track_id: row[2].get_str().unwrap_or("").to_string(),
        timestamp_ms: row[3].get_int().unwrap_or(0),
        timestamp_end_ms: row[4].get_int().unwrap_or(0),
        frame_hash: row[5].get_str().unwrap_or("").to_string(),
        perceptual_hash: row[6].get_str().unwrap_or("").to_string(),
        image_artifact_id: row[7].get_str().unwrap_or("").to_string(),
        ocr_text: row[8].get_str().unwrap_or("").to_string(),
        vlm_description: row[9].get_str().unwrap_or("").to_string(),
        provider: row[10].get_str().unwrap_or("").to_string(),
        model: row[11].get_str().unwrap_or("").to_string(),
        cost_usd: row[12].get_float().unwrap_or(0.0),
        chunk_id: row[13].get_str().unwrap_or("").to_string(),
        dedupe_group_id: row[14].get_str().unwrap_or("").to_string(),
        status: row[15].get_str().unwrap_or("").to_string(),
        warning: row[16].get_str().unwrap_or("").to_string(),
        created_at: row[17].get_str().unwrap_or("").to_string(),
    }
}

fn row_to_transcript_segment(row: &[DataValue]) -> TranscriptSegment {
    TranscriptSegment {
        segment_id: row[0].get_str().unwrap_or("").to_string(),
        video_id: row[1].get_str().unwrap_or("").to_string(),
        track_id: row[2].get_str().unwrap_or("").to_string(),
        start_ms: row[3].get_int().unwrap_or(0),
        end_ms: row[4].get_int().unwrap_or(0),
        speaker: row[5].get_str().unwrap_or("").to_string(),
        text: row[6].get_str().unwrap_or("").to_string(),
        confidence: row[7].get_float().unwrap_or(-1.0),
        source_method: row[8].get_str().unwrap_or("").to_string(),
        chunk_id: row[9].get_str().unwrap_or("").to_string(),
        created_at: row[10].get_str().unwrap_or("").to_string(),
    }
}
