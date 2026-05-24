use std::path::{Path, PathBuf};

use tokio::process::Command;

use crate::errors::VideoError;

pub(crate) async fn capture_caption_bytes(
    source_url: &str,
    downloader_bin: &str,
) -> Result<Option<Vec<u8>>, VideoError> {
    let temp = tempfile::tempdir().map_err(|e| VideoError::AcquisitionFailed {
        message: format!("create caption capture temp dir: {e}"),
    })?;
    let output = Command::new(downloader_bin)
        .args([
            "--skip-download",
            "--write-subs",
            "--write-auto-subs",
            "--sub-lang",
            "en.*,en",
            "--sub-format",
            "vtt",
            "--paths",
        ])
        .arg(temp.path())
        .args(["-o", "archon-caption-%(id)s.%(ext)s"])
        .arg(source_url)
        .output()
        .await
        .map_err(|_| VideoError::BinaryNotFound {
            name: "yt-dlp".into(),
            path: downloader_bin.into(),
        })?;

    if !output.status.success() {
        return Err(VideoError::AcquisitionFailed {
            message: format!(
                "caption capture failed: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            ),
        });
    }

    let Some(path) = largest_caption_file(temp.path())? else {
        return Ok(None);
    };
    tokio::fs::read(&path)
        .await
        .map(Some)
        .map_err(|e| VideoError::AcquisitionFailed {
            message: format!("read captured caption {}: {e}", path.display()),
        })
}

fn largest_caption_file(dir: &Path) -> Result<Option<PathBuf>, VideoError> {
    let mut candidates = std::fs::read_dir(dir)
        .map_err(|e| VideoError::AcquisitionFailed {
            message: format!("read caption capture dir: {e}"),
        })?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            path.extension()
                .and_then(|ext| ext.to_str())
                .is_some_and(|ext| {
                    matches!(ext.to_ascii_lowercase().as_str(), "vtt" | "srt" | "ttml")
                })
        })
        .filter_map(|path| {
            let len = path.metadata().ok()?.len();
            Some((len, path))
        })
        .collect::<Vec<_>>();
    candidates.sort_by(|left, right| right.0.cmp(&left.0));
    Ok(candidates.into_iter().map(|(_, path)| path).next())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::io::Write;
    use std::os::unix::fs::PermissionsExt;

    #[tokio::test]
    async fn captures_largest_vtt_from_downloader() {
        let dir = tempfile::tempdir().unwrap();
        let script = dir.path().join("yt-dlp-captions.sh");
        write_executable(
            &script,
            r#"#!/bin/sh
outdir=""
prev=""
for arg do
  if [ "$prev" = "--paths" ]; then outdir="$arg"; fi
  prev="$arg"
done
printf 'WEBVTT\n\n00:00:00.000 --> 00:00:01.000\nshort\n' > "$outdir/small.vtt"
printf 'WEBVTT\n\n00:00:00.000 --> 00:00:02.000\nlong caption text\n' > "$outdir/large.vtt"
"#,
        );

        let bytes =
            capture_caption_bytes("https://youtu.be/example", &script.display().to_string())
                .await
                .unwrap()
                .unwrap();

        let text = String::from_utf8(bytes).unwrap();
        assert!(text.contains("long caption text"));
    }

    fn write_executable(path: &Path, body: &str) {
        let mut file = fs::File::create(path).unwrap();
        file.write_all(body.as_bytes()).unwrap();
        let mut perms = fs::metadata(path).unwrap().permissions();
        perms.set_mode(0o755);
        fs::set_permissions(path, perms).unwrap();
    }
}
