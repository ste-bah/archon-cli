use std::collections::BTreeMap;
use std::path::Path;

use anyhow::{Result, bail};
use cozo::{DataValue, DbInstance, ScriptMutability};

pub(crate) fn delete_video(db: &DbInstance, video_id: &str, yes: bool) -> Result<()> {
    if !yes {
        bail!("refusing to delete video {video_id} without --yes");
    }
    let Some(source) = archon_video::store::get_video_source(db, video_id)? else {
        bail!("video not found: {video_id}");
    };
    let chunks = archon_docs::store::list_chunks_for_doc(db, &source.document_id)?.len();
    let frames = archon_video::store::list_frame_descriptions_for_video(db, video_id)?.len();
    delete_video_rows(db, video_id, &source.document_id)?;
    delete_video_artifacts(video_id, &source.local_path)?;
    println!(
        "Deleted video: {video_id} (document {}, {chunks} doc chunk(s), {frames} frame row(s))",
        source.document_id
    );
    Ok(())
}

fn delete_video_rows(db: &DbInstance, video_id: &str, document_id: &str) -> Result<()> {
    let mut params = BTreeMap::new();
    params.insert("vid".into(), DataValue::from(video_id));
    params.insert("did".into(), DataValue::from(document_id));

    run_rm_optional(
        db,
        "?[chunk_id] := *doc_chunks{chunk_id, document_id}, document_id = $did
         :rm vec_text_chunks { chunk_id }",
        params.clone(),
        "vec_text_chunks",
    )?;
    for (relation, key) in [
        ("doc_artifacts", "artifact_id"),
        ("doc_pages", "page_id"),
        ("doc_chunks", "chunk_id"),
    ] {
        for edge_column in ["from_artifact_id", "to_artifact_id"] {
            run_rm(
                db,
                &format!(
                    "?[edge_id] := *{relation}{{{key}: target_id, document_id}}, document_id = $did,
                     *doc_provenance_edges{{edge_id, {edge_column}: target_id}}
                     :rm doc_provenance_edges {{ edge_id }}"
                ),
                params.clone(),
                "doc_provenance_edges",
            )?;
        }
    }
    for edge_column in ["from_artifact_id", "to_artifact_id"] {
        run_rm(
            db,
            &format!(
                "?[edge_id] := *doc_provenance_edges{{edge_id, {edge_column}}}, {edge_column} = $did
                 :rm doc_provenance_edges {{ edge_id }}"
            ),
            params.clone(),
            "doc_provenance_edges",
        )?;
    }
    run_rm(
        db,
        "?[chunk_id] := *video_chunk_timeref{chunk_id, video_id}, video_id = $vid
         :rm video_chunk_timeref { chunk_id }",
        params.clone(),
        "video_chunk_timeref",
    )?;
    run_rm(
        db,
        "?[segment_id] := *video_transcript_segments{segment_id, video_id}, video_id = $vid
         :rm video_transcript_segments { segment_id }",
        params.clone(),
        "video_transcript_segments",
    )?;
    run_rm(
        db,
        "?[frame_id] := *video_frame_descriptions{frame_id, video_id}, video_id = $vid
         :rm video_frame_descriptions { frame_id }",
        params.clone(),
        "video_frame_descriptions",
    )?;
    run_rm(
        db,
        "?[track_id] := *video_tracks{track_id, video_id}, video_id = $vid
         :rm video_tracks { track_id }",
        params.clone(),
        "video_tracks",
    )?;
    run_rm(
        db,
        "?[video_id] <- [[$vid]]
         :rm video_sources { video_id }",
        params.clone(),
        "video_sources",
    )?;
    for (relation, key) in [
        ("doc_image_descriptions", "artifact_id"),
        ("doc_pdf_metrics", "document_id"),
        ("doc_processing_jobs", "job_id"),
        ("doc_ocr_runs", "ocr_run_id"),
        ("doc_pages", "page_id"),
        ("doc_artifacts", "artifact_id"),
        ("doc_chunks", "chunk_id"),
        ("doc_sources", "document_id"),
    ] {
        let script = if key == "document_id" {
            format!(
                "?[document_id] <- [[$did]]
                 :rm {relation} {{ document_id }}"
            )
        } else {
            format!(
                "?[{key}] := *{relation}{{{key}, document_id}}, document_id = $did
                 :rm {relation} {{ {key} }}"
            )
        };
        run_rm(db, &script, params.clone(), relation)?;
    }
    Ok(())
}

fn run_rm(
    db: &DbInstance,
    script: &str,
    params: BTreeMap<String, DataValue>,
    label: &str,
) -> Result<()> {
    db.run_script(script, params, ScriptMutability::Mutable)
        .map_err(|e| anyhow::anyhow!("delete {label} rows failed: {e}"))?;
    Ok(())
}

fn run_rm_optional(
    db: &DbInstance,
    script: &str,
    params: BTreeMap<String, DataValue>,
    label: &str,
) -> Result<()> {
    match db.run_script(script, params, ScriptMutability::Mutable) {
        Ok(_) => Ok(()),
        Err(e)
            if e.to_string()
                .contains(archon_docs::errors::COZO_RELATION_NOT_FOUND) =>
        {
            Ok(())
        }
        Err(e) => Err(anyhow::anyhow!("delete {label} rows failed: {e}")),
    }
}

fn delete_video_artifacts(video_id: &str, local_path: &str) -> Result<()> {
    let cwd = std::env::current_dir()?;
    let artifacts_root = cwd.join(".archon").join("video-artifacts");
    let video_dir = artifacts_root.join(video_id);
    if video_dir.exists() {
        std::fs::remove_dir_all(&video_dir)?;
    }
    if !local_path.trim().is_empty() {
        let path = Path::new(local_path);
        if path.starts_with(&artifacts_root) && path.exists() {
            std::fs::remove_file(path)?;
        }
    }
    Ok(())
}
