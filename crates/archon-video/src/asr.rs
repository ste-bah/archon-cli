use std::io::Write;
use std::path::{Path, PathBuf};

use archon_policy::VideoPolicy;
use async_trait::async_trait;
use serde_json::Value;
use tempfile::{Builder, NamedTempFile};
use tokio::process::Command;

use crate::errors::VideoError;
use crate::transcript::TranscriptSegment;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AsrOptions {
    pub model: String,
    pub device: String,
    pub language: Option<String>,
    pub vad_stable_timestamps: bool,
    pub diarization: bool,
}

impl From<&VideoPolicy> for AsrOptions {
    fn from(policy: &VideoPolicy) -> Self {
        Self {
            model: policy.asr.model.clone(),
            device: policy.asr.device.clone(),
            language: None,
            vad_stable_timestamps: policy.asr.vad_stable_timestamps,
            diarization: policy.asr.diarization,
        }
    }
}

#[async_trait]
pub trait AsrProvider: Send + Sync {
    async fn transcribe(
        &self,
        audio_bytes: &[u8],
        opts: &AsrOptions,
    ) -> Result<Vec<TranscriptSegment>, VideoError>;

    fn provider_name(&self) -> &str;
}

pub async fn extract_audio_track(
    video_path: &Path,
    ffmpeg_bin: &str,
) -> Result<NamedTempFile, VideoError> {
    let bin = std::env::var("ARCHON_FFMPEG_BIN").unwrap_or_else(|_| ffmpeg_bin.to_string());
    let tmp = Builder::new()
        .prefix("archon-video-audio-")
        .suffix(".wav")
        .tempfile()
        .map_err(|e| VideoError::AcquisitionFailed {
            message: format!("create temp wav: {e}"),
        })?;
    let output = Command::new(&bin)
        .args(["-hide_banner", "-nostdin", "-y"])
        .arg("-i")
        .arg(video_path)
        .args(["-vn", "-ar", "16000", "-ac", "1", "-f", "wav"])
        .arg(tmp.path())
        .output()
        .await
        .map_err(|_| VideoError::BinaryNotFound {
            name: "ffmpeg".into(),
            path: bin.clone(),
        })?;
    if !output.status.success() {
        return Err(VideoError::MetadataFailed {
            message: format!(
                "ffmpeg audio extraction failed: {}",
                String::from_utf8_lossy(&output.stderr)
            ),
        });
    }
    Ok(tmp)
}

pub struct MockAsrAdapter {
    pub segments: Vec<TranscriptSegment>,
}

#[async_trait]
impl AsrProvider for MockAsrAdapter {
    async fn transcribe(
        &self,
        _audio_bytes: &[u8],
        _opts: &AsrOptions,
    ) -> Result<Vec<TranscriptSegment>, VideoError> {
        Ok(self.segments.clone())
    }

    fn provider_name(&self) -> &str {
        "mock_asr"
    }
}

pub struct NullAsrAdapter {
    pub name: String,
    pub message: String,
}

impl Default for NullAsrAdapter {
    fn default() -> Self {
        Self {
            name: "unavailable".into(),
            message: "ASR provider not available".into(),
        }
    }
}

#[async_trait]
impl AsrProvider for NullAsrAdapter {
    async fn transcribe(
        &self,
        _audio_bytes: &[u8],
        _opts: &AsrOptions,
    ) -> Result<Vec<TranscriptSegment>, VideoError> {
        Err(VideoError::AsrProviderUnavailable {
            message: self.message.clone(),
        })
    }

    fn provider_name(&self) -> &str {
        &self.name
    }
}

pub struct WhisperCppAdapter {
    pub bin: String,
}

#[async_trait]
impl AsrProvider for WhisperCppAdapter {
    async fn transcribe(
        &self,
        audio_bytes: &[u8],
        opts: &AsrOptions,
    ) -> Result<Vec<TranscriptSegment>, VideoError> {
        let mut tmp = Builder::new()
            .prefix("archon-whisper-cpp-")
            .suffix(".wav")
            .tempfile()
            .map_err(|e| VideoError::AsrProviderUnavailable {
                message: format!("create whisper-cpp input: {e}"),
            })?;
        tmp.write_all(audio_bytes)
            .map_err(|e| VideoError::AsrProviderUnavailable {
                message: format!("write whisper-cpp input: {e}"),
            })?;
        let out_dir = tempfile::tempdir().map_err(|e| VideoError::AsrProviderUnavailable {
            message: format!("create whisper-cpp output directory: {e}"),
        })?;
        let output_prefix = out_dir.path().join("transcript");
        let output = Command::new(&self.bin)
            .args(["--model", &opts.model, "--output-json", "--output-file"])
            .arg(&output_prefix)
            .arg("--file")
            .arg(tmp.path())
            .output()
            .await
            .map_err(|_| VideoError::BinaryNotFound {
                name: "whisper-cli".into(),
                path: self.bin.clone(),
            })?;
        if !output.status.success() {
            return Err(VideoError::AsrProviderUnavailable {
                message: String::from_utf8_lossy(&output.stderr).to_string(),
            });
        }
        let json_path = output_prefix.with_extension("json");
        let json = std::fs::read(&json_path).map_err(|e| VideoError::AsrProviderUnavailable {
            message: format!(
                "whisper-cpp succeeded but JSON output was not readable at {}: {e}",
                json_path.display()
            ),
        })?;
        parse_whisper_cpp_json(&json)
    }

    fn provider_name(&self) -> &str {
        "whisper-cpp"
    }
}

pub fn parse_whisper_cpp_json(json: &[u8]) -> Result<Vec<TranscriptSegment>, VideoError> {
    let value: Value =
        serde_json::from_slice(json).map_err(|e| VideoError::AsrProviderUnavailable {
            message: format!("parse whisper-cpp JSON: {e}"),
        })?;
    let Some(items) = value.get("transcription").and_then(Value::as_array) else {
        return Ok(Vec::new());
    };
    let mut segments = Vec::new();
    for item in items {
        let text = item
            .get("text")
            .and_then(Value::as_str)
            .unwrap_or("")
            .trim()
            .to_string();
        if text.is_empty() {
            continue;
        }
        let (start_ms, end_ms) = whisper_segment_offsets(item).unwrap_or((0, 100));
        segments.push(TranscriptSegment {
            start_ms,
            end_ms: end_ms.max(start_ms + 100),
            text,
            confidence: None,
            speaker: None,
        });
    }
    Ok(segments)
}

fn whisper_segment_offsets(item: &Value) -> Option<(u64, u64)> {
    item.get("offsets")
        .and_then(|offsets| Some((offsets.get("from")?.as_u64()?, offsets.get("to")?.as_u64()?)))
        .or_else(|| {
            let timestamps = item.get("timestamps")?;
            Some((
                parse_whisper_timestamp(timestamps.get("from")?.as_str()?)?,
                parse_whisper_timestamp(timestamps.get("to")?.as_str()?)?,
            ))
        })
}

fn parse_whisper_timestamp(value: &str) -> Option<u64> {
    let mut parts = value
        .replace(',', ".")
        .split(':')
        .map(str::to_string)
        .collect::<Vec<_>>();
    if parts.len() != 3 {
        return None;
    }
    let seconds = parts.pop()?.parse::<f64>().ok()?;
    let minutes = parts.pop()?.parse::<u64>().ok()?;
    let hours = parts.pop()?.parse::<u64>().ok()?;
    Some(hours * 3_600_000 + minutes * 60_000 + (seconds * 1000.0).round() as u64)
}

pub struct FasterWhisperAdapter {
    pub bin: String,
}

#[async_trait]
impl AsrProvider for FasterWhisperAdapter {
    async fn transcribe(
        &self,
        _audio_bytes: &[u8],
        _opts: &AsrOptions,
    ) -> Result<Vec<TranscriptSegment>, VideoError> {
        Err(VideoError::AsrProviderUnavailable {
            message: format!(
                "faster-whisper adapter is not wired for binary {}",
                self.bin
            ),
        })
    }

    fn provider_name(&self) -> &str {
        "faster-whisper"
    }
}

pub struct CloudAsrAdapter;

#[async_trait]
impl AsrProvider for CloudAsrAdapter {
    async fn transcribe(
        &self,
        _audio_bytes: &[u8],
        _opts: &AsrOptions,
    ) -> Result<Vec<TranscriptSegment>, VideoError> {
        Err(VideoError::AsrProviderUnavailable {
            message: "cloud ASR is policy-gated and not implemented yet".into(),
        })
    }

    fn provider_name(&self) -> &str {
        "cloud"
    }
}

pub fn select_asr_provider(policy: &VideoPolicy) -> Box<dyn AsrProvider> {
    match policy.asr.provider.as_str() {
        "whisper-rs" => {
            let (device, fallback) = probe_gpu_backend(&policy.asr.device);
            if fallback {
                tracing::warn!(
                    "requested ASR GPU backend was unavailable; using CPU for whisper-rs"
                );
            }
            tracing::warn!(
                "whisper-rs ASR is unavailable in this build path; set provider=\"whisper-cpp\" explicitly to use a whisper-cli subprocess"
            );
            Box::new(NullAsrAdapter {
                name: format!("whisper-rs/{device}"),
                message: "ASR provider not available: whisper-rs is unavailable in this build path"
                    .into(),
            })
        }
        "whisper-cpp" => Box::new(WhisperCppAdapter {
            bin: std::env::var("ARCHON_WHISPER_BIN").unwrap_or_else(|_| "whisper-cli".into()),
        }),
        "faster-whisper" => Box::new(FasterWhisperAdapter {
            bin: std::env::var("ARCHON_FASTER_WHISPER_BIN")
                .unwrap_or_else(|_| "faster-whisper".into()),
        }),
        "cloud" => Box::new(CloudAsrAdapter),
        "disabled" | "" => Box::new(NullAsrAdapter::default()),
        other => {
            tracing::warn!("unknown ASR provider '{other}', using unavailable adapter");
            Box::new(NullAsrAdapter::default())
        }
    }
}

pub fn enforce_monotonic_boundaries(segments: &mut [TranscriptSegment]) {
    for index in 0..segments.len() {
        if segments[index].end_ms <= segments[index].start_ms {
            segments[index].end_ms = segments[index].start_ms + 100;
        }
        if index == 0 {
            continue;
        }
        let previous_end = segments[index - 1].end_ms;
        if segments[index].start_ms < previous_end {
            segments[index].start_ms = previous_end;
        }
        if segments[index].end_ms <= segments[index].start_ms {
            segments[index].end_ms = segments[index].start_ms + 100;
        }
    }
}

#[async_trait]
pub trait DiarizationProvider: Send + Sync {
    async fn attribute_speakers(
        &self,
        segments: Vec<TranscriptSegment>,
        audio_bytes: &[u8],
    ) -> Result<Vec<TranscriptSegment>, VideoError>;

    fn provider_name(&self) -> &str;
}

pub struct MockDiarizerProvider;

#[async_trait]
impl DiarizationProvider for MockDiarizerProvider {
    async fn attribute_speakers(
        &self,
        mut segments: Vec<TranscriptSegment>,
        _audio_bytes: &[u8],
    ) -> Result<Vec<TranscriptSegment>, VideoError> {
        for (index, segment) in segments.iter_mut().enumerate() {
            segment.speaker = Some(if index % 2 == 0 {
                "SPEAKER_A".into()
            } else {
                "SPEAKER_B".into()
            });
        }
        Ok(segments)
    }

    fn provider_name(&self) -> &str {
        "mock_diarizer"
    }
}

pub struct NullDiarizerProvider;

#[async_trait]
impl DiarizationProvider for NullDiarizerProvider {
    async fn attribute_speakers(
        &self,
        segments: Vec<TranscriptSegment>,
        _audio_bytes: &[u8],
    ) -> Result<Vec<TranscriptSegment>, VideoError> {
        Ok(segments)
    }

    fn provider_name(&self) -> &str {
        "none"
    }
}

pub fn select_diarizer_provider(_policy: &VideoPolicy) -> Box<dyn DiarizationProvider> {
    Box::new(NullDiarizerProvider)
}

pub async fn apply_diarization(
    segments: Vec<TranscriptSegment>,
    audio_bytes: &[u8],
    provider: &dyn DiarizationProvider,
) -> (Vec<TranscriptSegment>, Vec<String>) {
    let original = segments.clone();
    match provider.attribute_speakers(segments, audio_bytes).await {
        Ok(segments) => (segments, Vec::new()),
        Err(error) => (
            original,
            vec![format!(
                "diarization provider {} failed: {error}",
                provider.provider_name()
            )],
        ),
    }
}

pub fn probe_gpu_backend(requested: &str) -> (String, bool) {
    match requested.trim().to_ascii_lowercase().as_str() {
        "" | "auto" | "cpu" => ("cpu".into(), false),
        "cuda" | "metal" | "vulkan" | "coreml" => ("cpu".into(), true),
        other => {
            tracing::warn!("unknown ASR device '{other}', falling back to CPU");
            ("cpu".into(), true)
        }
    }
}

pub fn resolve_model_path(
    model_source: &str,
    model_cache_dir: &str,
    model: &str,
    policy: &VideoPolicy,
) -> Result<PathBuf, VideoError> {
    if !model_source.trim().is_empty() {
        return resolve_model_source(model_source, model_cache_dir, policy);
    }
    let cache_dir = default_model_cache_dir(model_cache_dir);
    let model_file = cache_dir.join(format!("{model}.bin"));
    if model_file.exists() {
        return Ok(model_file);
    }
    if !policy.allow_cloud_asr {
        return Err(VideoError::PolicyDenied {
            reason: format!(
                "model {model} was not found in {}; model download requires allow_cloud_asr = true",
                cache_dir.display()
            ),
        });
    }
    Err(VideoError::AsrProviderUnavailable {
        message: "model download is policy-gated but not implemented yet".into(),
    })
}

fn resolve_model_source(
    model_source: &str,
    model_cache_dir: &str,
    policy: &VideoPolicy,
) -> Result<PathBuf, VideoError> {
    if model_source.starts_with("http://") || model_source.starts_with("https://") {
        if policy.allow_cloud_asr {
            return Err(VideoError::AsrProviderUnavailable {
                message: "model URL download is not implemented yet".into(),
            });
        }
        return Err(VideoError::PolicyDenied {
            reason: "model_source URL requires allow_cloud_asr = true".into(),
        });
    }
    let path = expand_tilde(model_source);
    if path.exists() {
        return Ok(path);
    }
    let cache_path = default_model_cache_dir(model_cache_dir).join(model_source);
    if cache_path.exists() {
        return Ok(cache_path);
    }
    Err(VideoError::AsrProviderUnavailable {
        message: format!("model_source path not found: {model_source}"),
    })
}

fn default_model_cache_dir(model_cache_dir: &str) -> PathBuf {
    if !model_cache_dir.trim().is_empty() {
        return expand_tilde(model_cache_dir);
    }
    dirs::home_dir()
        .map(|home| home.join(".archon/models/whisper"))
        .unwrap_or_else(|| PathBuf::from(".archon/models/whisper"))
}

fn expand_tilde(path: &str) -> PathBuf {
    if let Some(rest) = path.strip_prefix("~/")
        && let Some(home) = dirs::home_dir()
    {
        return home.join(rest);
    }
    PathBuf::from(path)
}
