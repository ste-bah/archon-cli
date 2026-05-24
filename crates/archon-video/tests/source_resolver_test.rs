use archon_policy::EffectivePolicy;
use archon_video::errors::VideoError;
use archon_video::source::{
    AcquisitionMethod, ResolveOpts, TranscriptionSource, VideoSourceKind, parse_youtube_video_id,
    resolve_source,
};
use serde_json::Value;

fn video_policy() -> EffectivePolicy {
    let mut policy = EffectivePolicy::default();
    policy.video.enabled = true;
    policy
}

#[test]
fn youtube_url_forms_extract_video_id_and_reject_playlists() {
    assert_eq!(
        parse_youtube_video_id("https://www.youtube.com/watch?v=abc123").unwrap(),
        Some("abc123".into())
    );
    assert_eq!(
        parse_youtube_video_id("https://youtu.be/abc123").unwrap(),
        Some("abc123".into())
    );

    let err = parse_youtube_video_id("https://www.youtube.com/watch?v=abc123&list=PL1")
        .expect_err("playlist URLs should be rejected");
    assert!(matches!(err, VideoError::PlaylistRejected { .. }));
}

#[test]
fn resolver_accepts_youtube_when_policy_selects_downloader() {
    let mut policy = video_policy();
    policy.video.allow_youtube = true;
    policy.video.allow_external_downloaders = true;

    let resolved = resolve_source(
        "https://www.youtube.com/watch?v=abc123",
        &ResolveOpts::default(),
        &policy,
    )
    .unwrap();

    assert_eq!(resolved.source_kind, VideoSourceKind::YouTube);
    assert_eq!(resolved.video_id.as_deref(), Some("abc123"));
    assert_eq!(
        resolved.acquisition_method,
        AcquisitionMethod::ExternalDownloader
    );
}

#[test]
fn resolver_records_caption_plan_when_policy_prefers_captions() {
    let mut policy = video_policy();
    policy.video.allow_youtube = true;
    policy.video.allow_external_downloaders = true;
    policy.video.allow_caption_capture = true;
    let opts = ResolveOpts {
        prefer_caption: true,
        ..Default::default()
    };

    let resolved = resolve_source("https://youtu.be/abc123", &opts, &policy).unwrap();

    assert_eq!(
        resolved.transcription_source_plan,
        TranscriptionSource::CapturedCaption
    );
}

#[test]
fn resolver_rejects_ftp_accepts_local_and_direct_url() {
    let mut policy = video_policy();
    policy.video.allow_direct_urls = true;

    let ftp = resolve_source(
        "ftp://example.com/video.mp4",
        &ResolveOpts::default(),
        &policy,
    )
    .expect_err("ftp should be rejected");
    assert!(matches!(ftp, VideoError::UnsupportedScheme { .. }));

    let local = resolve_source("./local.mp4", &ResolveOpts::default(), &policy).unwrap();
    assert_eq!(local.source_kind, VideoSourceKind::LocalFile);
    assert_eq!(local.acquisition_method, AcquisitionMethod::LocalFile);

    let direct = resolve_source(
        "https://cdn.example.com/video.mp4",
        &ResolveOpts::default(),
        &policy,
    )
    .unwrap();
    assert_eq!(direct.source_kind, VideoSourceKind::DirectUrl);
    assert_eq!(direct.acquisition_method, AcquisitionMethod::DirectDownload);
}

#[test]
fn resolver_records_policy_snapshot_json() {
    let policy = video_policy();
    let opts = ResolveOpts {
        transcript_path: Some("transcript.vtt".into()),
        ..Default::default()
    };

    let resolved = resolve_source("./local.mp4", &opts, &policy).unwrap();
    let snapshot: Value = serde_json::from_str(&resolved.policy_snapshot_json).unwrap();

    assert_eq!(
        resolved.transcription_source_plan,
        TranscriptionSource::UserTranscript
    );
    assert_eq!(snapshot["acquisition_method"], "LocalFile");
    assert_eq!(snapshot["transcription_source"], "UserTranscript");
    assert_eq!(snapshot["source_kind"], "LocalFile");
}
