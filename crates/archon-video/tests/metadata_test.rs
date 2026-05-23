use std::io::Write;

use archon_video::errors::VideoError;
use archon_video::metadata::{MetadataOpts, extract_metadata};

#[tokio::test]
async fn extract_metadata_parses_mock_ffprobe_json() {
    let dir = tempfile::tempdir().unwrap();
    let script = dir.path().join("ffprobe-mock.sh");
    write_script(
        &script,
        r#"#!/bin/sh
printf '%s\n' '{"format":{"duration":"12.345","format_name":"mov,mp4","tags":{"title":"Demo","artist":"Archon","creation_time":"2026-05-22T10:00:00Z"}},"streams":[{"codec_name":"h264"}]}'
"#,
    );

    let metadata = extract_metadata(
        "fixture.mp4",
        &MetadataOpts {
            ffprobe_bin: script.display().to_string(),
            timeout_secs: 5,
        },
    )
    .await
    .unwrap();

    assert_eq!(metadata.duration_ms, Some(12_345));
    assert_eq!(metadata.title.as_deref(), Some("Demo"));
    assert_eq!(metadata.channel_or_author.as_deref(), Some("Archon"));
    assert_eq!(metadata.format_name.as_deref(), Some("mov,mp4"));
    assert_eq!(metadata.codec.as_deref(), Some("h264"));
}

#[tokio::test]
async fn extract_metadata_reports_missing_ffprobe() {
    let err = extract_metadata(
        "fixture.mp4",
        &MetadataOpts {
            ffprobe_bin: "__archon_missing_ffprobe__".into(),
            timeout_secs: 1,
        },
    )
    .await
    .unwrap_err();

    assert!(matches!(err, VideoError::BinaryNotFound { .. }));
}

#[tokio::test]
async fn extract_metadata_reports_nonzero_ffprobe_stderr() {
    let dir = tempfile::tempdir().unwrap();
    let script = dir.path().join("ffprobe-fail.sh");
    write_script(
        &script,
        r#"#!/bin/sh
printf '%s\n' 'bad media' >&2
exit 7
"#,
    );

    let err = extract_metadata(
        "fixture.mp4",
        &MetadataOpts {
            ffprobe_bin: script.display().to_string(),
            timeout_secs: 5,
        },
    )
    .await
    .unwrap_err();

    assert!(matches!(err, VideoError::MetadataFailed { .. }));
    assert!(err.to_string().contains("bad media"));
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
