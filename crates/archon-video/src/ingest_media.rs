use std::path::Path;

use archon_policy::EffectivePolicy;
use cozo::DbInstance;

use crate::acquire::{
    AcquireOptions, AcquiredMedia, AcquisitionAdapter, ExternalDownloaderAdapter,
};
use crate::asr::extract_audio_track;
use crate::errors::VideoError;
use crate::ingest::IngestOpts;
use crate::source::{AcquisitionMethod, VideoSourceResolution};
use crate::store;

pub(crate) async fn acquire_media_if_needed(
    opts: &IngestOpts,
    policy: &EffectivePolicy,
    resolution: &VideoSourceResolution,
) -> Result<Option<AcquiredMedia>, VideoError> {
    if resolution.local_path.is_some() || opts.metadata_only {
        return Ok(None);
    }
    match resolution.acquisition_method {
        AcquisitionMethod::ExternalDownloader => {
            let adapter = ExternalDownloaderAdapter::from_policy(&policy.video);
            adapter
                .acquire(
                    &resolution.source_url,
                    &AcquireOptions {
                        policy: policy.clone(),
                        audio_only: !wants_frame_evidence(opts, policy),
                        yes: opts.yes,
                    },
                )
                .await
                .map(Some)
        }
        AcquisitionMethod::None | AcquisitionMethod::LocalFile => Ok(None),
        AcquisitionMethod::BrowserAutomation => Err(VideoError::AcquisitionFailed {
            message: "browser automation acquisition is not implemented yet".into(),
        }),
        AcquisitionMethod::DirectDownload => Err(VideoError::AcquisitionFailed {
            message: "direct video URL download is not implemented yet".into(),
        }),
    }
}

pub(crate) async fn asr_audio_bytes(
    media_path: Option<&Path>,
    allow_missing_for_mock: bool,
) -> Result<Vec<u8>, VideoError> {
    let Some(path) = media_path else {
        return Ok(Vec::new());
    };
    if !path.exists() {
        if allow_missing_for_mock {
            return Ok(Vec::new());
        }
        return Err(VideoError::SourceNotFound {
            path: path.display().to_string(),
        });
    }
    let wav;
    let audio_path = if path
        .extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| ext.eq_ignore_ascii_case("wav"))
    {
        path
    } else {
        wav = extract_audio_track(path, "ffmpeg").await?;
        wav.path()
    };
    tokio::fs::read(audio_path)
        .await
        .map_err(|e| VideoError::AcquisitionFailed {
            message: format!("read ASR audio {}: {e}", audio_path.display()),
        })
}

pub(crate) fn successful_video_by_hash(
    db: &DbInstance,
    source_hash: &str,
) -> Result<Option<store::VideoSource>, VideoError> {
    Ok(store::list_video_sources(db)?
        .into_iter()
        .find(|source| source.source_hash == source_hash && source.ingest_status == "success"))
}

fn wants_frame_evidence(opts: &IngestOpts, policy: &EffectivePolicy) -> bool {
    !opts
        .frames_mode
        .as_deref()
        .unwrap_or(policy.video.frames.mode.as_str())
        .eq_ignore_ascii_case("none")
}
