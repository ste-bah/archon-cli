use std::io::Write;

use archon_policy::EffectivePolicy;
use archon_video::acquire::{
    AcquireOptions, AcquisitionAdapter, ExternalDownloaderAdapter, MockAcquisitionAdapter,
};
use archon_video::errors::VideoError;
use archon_video::source::AcquisitionMethod;

#[tokio::test]
async fn external_downloader_is_denied_by_default_policy() {
    let adapter = ExternalDownloaderAdapter {
        bin: "__not_called__".into(),
    };
    let err = adapter
        .acquire(
            "https://example.test/video",
            &AcquireOptions {
                policy: EffectivePolicy::default(),
                audio_only: true,
                yes: true,
            },
        )
        .await
        .unwrap_err();

    assert!(matches!(err, VideoError::PolicyDenied { .. }));
}

#[tokio::test]
async fn mock_acquisition_adapter_returns_fixture_path() {
    let fixture_path = std::path::PathBuf::from("tests/fixtures/mini_lecture.mp4");
    let adapter = MockAcquisitionAdapter {
        fixture_path: fixture_path.clone(),
        method: AcquisitionMethod::ExternalDownloader,
    };
    let media = adapter
        .acquire(
            "https://example.test/video",
            &AcquireOptions {
                policy: EffectivePolicy::default(),
                audio_only: true,
                yes: true,
            },
        )
        .await
        .unwrap();

    assert_eq!(media.local_path, fixture_path);
    assert_eq!(
        media.acquisition_method,
        AcquisitionMethod::ExternalDownloader
    );
}

#[tokio::test]
async fn external_downloader_reports_platform_blocks_honestly() {
    let dir = tempfile::tempdir().unwrap();
    let script = dir.path().join("yt-dlp-mock.sh");
    write_script(
        &script,
        r#"#!/bin/sh
printf '%s\n' 'Sign in to confirm you are not a bot' >&2
exit 1
"#,
    );
    let mut policy = EffectivePolicy::default();
    policy.video.enabled = true;
    policy.video.allow_external_downloaders = true;
    policy.video.require_user_confirmation_for_download = false;

    let err = ExternalDownloaderAdapter {
        bin: script.display().to_string(),
    }
    .acquire(
        "https://example.test/video",
        &AcquireOptions {
            policy,
            audio_only: true,
            yes: true,
        },
    )
    .await
    .unwrap_err();

    assert!(matches!(err, VideoError::AcquisitionFailed { .. }));
    assert!(err.to_string().contains("does not attempt to bypass"));
}

fn write_script(path: &std::path::Path, body: &str) {
    let mut file = std::fs::File::create(path).unwrap();
    file.write_all(body.as_bytes()).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
}
