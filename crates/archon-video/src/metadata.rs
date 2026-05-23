use std::time::Duration;

use serde_json::Value;
use tokio::process::Command;

use crate::errors::VideoError;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct VideoMetadata {
    pub duration_ms: Option<u64>,
    pub title: Option<String>,
    pub channel_or_author: Option<String>,
    pub published_at: Option<String>,
    pub format_name: Option<String>,
    pub codec: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MetadataOpts {
    pub ffprobe_bin: String,
    pub timeout_secs: u32,
}

impl Default for MetadataOpts {
    fn default() -> Self {
        Self {
            ffprobe_bin: std::env::var("ARCHON_FFPROBE_BIN").unwrap_or_else(|_| "ffprobe".into()),
            timeout_secs: 30,
        }
    }
}

pub async fn extract_metadata(
    path: &str,
    opts: &MetadataOpts,
) -> Result<VideoMetadata, VideoError> {
    let bin = resolve_binary(&opts.ffprobe_bin, "ARCHON_FFPROBE_BIN").ok_or_else(|| {
        VideoError::BinaryNotFound {
            name: "ffprobe".into(),
            path: opts.ffprobe_bin.clone(),
        }
    })?;
    let output = tokio::time::timeout(
        Duration::from_secs(opts.timeout_secs as u64),
        Command::new(&bin)
            .args([
                "-v",
                "quiet",
                "-print_format",
                "json",
                "-show_format",
                "-show_streams",
                path,
            ])
            .output(),
    )
    .await
    .map_err(|_| VideoError::MetadataFailed {
        message: format!("ffprobe timed out after {}s", opts.timeout_secs),
    })?
    .map_err(|_| VideoError::BinaryNotFound {
        name: "ffprobe".into(),
        path: bin.clone(),
    })?;

    if !output.status.success() {
        return Err(VideoError::MetadataFailed {
            message: String::from_utf8_lossy(&output.stderr).to_string(),
        });
    }
    parse_ffprobe_json(&output.stdout)
}

fn parse_ffprobe_json(bytes: &[u8]) -> Result<VideoMetadata, VideoError> {
    let json: Value = serde_json::from_slice(bytes).map_err(|e| VideoError::MetadataFailed {
        message: format!("parse ffprobe JSON: {e}"),
    })?;
    let format = &json["format"];
    let tags = &format["tags"];
    Ok(VideoMetadata {
        duration_ms: duration_ms(format.get("duration")),
        title: string_at(tags, "title"),
        channel_or_author: string_at(tags, "artist").or_else(|| string_at(tags, "author")),
        published_at: string_at(tags, "creation_time"),
        format_name: string_at(format, "format_name"),
        codec: json["streams"]
            .as_array()
            .and_then(|streams| streams.first())
            .and_then(|stream| string_at(stream, "codec_name")),
    })
}

fn duration_ms(value: Option<&Value>) -> Option<u64> {
    let seconds = value.and_then(|value| {
        value
            .as_str()
            .and_then(|s| s.parse::<f64>().ok())
            .or_else(|| value.as_f64())
    })?;
    Some((seconds * 1000.0).round() as u64)
}

fn string_at(value: &Value, key: &str) -> Option<String> {
    value
        .get(key)
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

fn resolve_binary(bin: &str, env_var: &str) -> Option<String> {
    let candidate = std::env::var(env_var).unwrap_or_else(|_| bin.to_string());
    which::which(&candidate)
        .ok()
        .map(|path| path.display().to_string())
}
