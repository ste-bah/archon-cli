use std::fmt;
use std::path::PathBuf;
use std::str::FromStr;

use archon_policy::EffectivePolicy;
use serde::{Deserialize, Serialize};
use url::Url;

use crate::errors::VideoError;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum VideoSourceKind {
    LocalFile,
    DirectUrl,
    YouTube,
    TranscriptOnly,
}

impl fmt::Display for VideoSourceKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::LocalFile => "LocalFile",
            Self::DirectUrl => "DirectUrl",
            Self::YouTube => "YouTube",
            Self::TranscriptOnly => "TranscriptOnly",
        })
    }
}

impl FromStr for VideoSourceKind {
    type Err = VideoError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "LocalFile" | "local-file" | "local_file" => Ok(Self::LocalFile),
            "DirectUrl" | "direct-url" | "direct_url" => Ok(Self::DirectUrl),
            "YouTube" | "youtube" => Ok(Self::YouTube),
            "TranscriptOnly" | "transcript-only" | "transcript_only" => Ok(Self::TranscriptOnly),
            other => Err(VideoError::SourceNotFound {
                path: other.to_string(),
            }),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum AcquisitionMethod {
    LocalFile,
    DirectDownload,
    BrowserAutomation,
    ExternalDownloader,
    None,
}

impl fmt::Display for AcquisitionMethod {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::LocalFile => "LocalFile",
            Self::DirectDownload => "DirectDownload",
            Self::BrowserAutomation => "BrowserAutomation",
            Self::ExternalDownloader => "ExternalDownloader",
            Self::None => "None",
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum TranscriptionSource {
    UserTranscript,
    LocalAsr,
    CapturedCaption,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ResolveOpts {
    pub transcript_path: Option<PathBuf>,
    pub metadata_only: bool,
    pub prefer_caption: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VideoSourceResolution {
    pub source_kind: VideoSourceKind,
    pub acquisition_method: AcquisitionMethod,
    pub transcription_source_plan: TranscriptionSource,
    pub source_url: String,
    pub local_path: Option<PathBuf>,
    pub video_id: Option<String>,
    pub policy_snapshot_json: String,
}

#[derive(Debug, Serialize)]
struct PolicySnapshot<'a> {
    acquisition_method: &'a AcquisitionMethod,
    transcription_source: &'a TranscriptionSource,
    source_kind: &'a VideoSourceKind,
}

pub fn resolve_source(
    input: &str,
    opts: &ResolveOpts,
    policy: &EffectivePolicy,
) -> Result<VideoSourceResolution, VideoError> {
    let ingest = policy.video_ingest_decision();
    if !ingest.allowed {
        return Err(VideoError::PolicyDenied {
            reason: ingest.reason,
        });
    }

    let classified = classify_input(input, opts)?;
    let acq_gate = policy.video_acquisition_decision(&classified.kind.to_string());
    if !acq_gate.allowed && classified.kind != VideoSourceKind::LocalFile {
        return Err(VideoError::PolicyDenied {
            reason: acq_gate.reason,
        });
    }

    let acquisition_method = choose_acquisition_method(&classified.kind, opts, policy)?;
    let method_gate = policy.video_acquisition_decision(&acquisition_method.to_string());
    if !method_gate.allowed && acquisition_method != AcquisitionMethod::LocalFile {
        return Err(VideoError::PolicyDenied {
            reason: method_gate.reason,
        });
    }

    let transcription_source_plan = choose_transcription_source(opts, policy);
    let policy_snapshot_json = serde_json::to_string(&PolicySnapshot {
        acquisition_method: &acquisition_method,
        transcription_source: &transcription_source_plan,
        source_kind: &classified.kind,
    })
    .map_err(|e| VideoError::AcquisitionFailed {
        message: format!("serialize policy snapshot: {e}"),
    })?;

    Ok(VideoSourceResolution {
        source_kind: classified.kind,
        acquisition_method,
        transcription_source_plan,
        source_url: input.to_string(),
        local_path: classified.local_path,
        video_id: classified.video_id,
        policy_snapshot_json,
    })
}

struct ClassifiedInput {
    kind: VideoSourceKind,
    local_path: Option<PathBuf>,
    video_id: Option<String>,
}

fn classify_input(input: &str, opts: &ResolveOpts) -> Result<ClassifiedInput, VideoError> {
    let trimmed = input.trim();
    if trimmed.is_empty() && opts.transcript_path.is_some() {
        return Ok(ClassifiedInput {
            kind: VideoSourceKind::TranscriptOnly,
            local_path: None,
            video_id: None,
        });
    }

    if let Some(video_id) = parse_youtube_video_id(trimmed)? {
        return Ok(ClassifiedInput {
            kind: VideoSourceKind::YouTube,
            local_path: None,
            video_id: Some(video_id),
        });
    }

    if let Ok(url) = Url::parse(trimmed) {
        return match url.scheme() {
            "http" | "https" => Ok(ClassifiedInput {
                kind: VideoSourceKind::DirectUrl,
                local_path: None,
                video_id: None,
            }),
            scheme => Err(VideoError::UnsupportedScheme {
                scheme: scheme.to_string(),
            }),
        };
    }

    if is_video_file_extension(trimmed) {
        return Ok(ClassifiedInput {
            kind: VideoSourceKind::LocalFile,
            local_path: Some(PathBuf::from(trimmed)),
            video_id: None,
        });
    }

    Err(VideoError::SourceNotFound {
        path: trimmed.to_string(),
    })
}

pub fn parse_youtube_video_id(input: &str) -> Result<Option<String>, VideoError> {
    let Ok(url) = Url::parse(input) else {
        return Ok(None);
    };
    let Some(host) = url.host_str().map(|host| host.to_ascii_lowercase()) else {
        return Ok(None);
    };
    let is_youtube = host.ends_with("youtube.com") || host == "youtu.be";
    if !is_youtube {
        return Ok(None);
    }
    if url.query_pairs().any(|(key, _)| key == "list")
        || url.path().contains("/playlist")
        || url.path().contains("/channel/")
    {
        return Err(VideoError::PlaylistRejected {
            url: input.to_string(),
        });
    }
    if host == "youtu.be" {
        return Ok(url
            .path_segments()
            .and_then(|mut segments| segments.next())
            .filter(|id| !id.is_empty())
            .map(str::to_string));
    }
    if url.path() == "/watch" {
        return Ok(url
            .query_pairs()
            .find(|(key, _)| key == "v")
            .map(|(_, value)| value.into_owned()));
    }
    Err(VideoError::AcquisitionFailed {
        message: "unsupported YouTube URL form".into(),
    })
}

fn choose_acquisition_method(
    kind: &VideoSourceKind,
    opts: &ResolveOpts,
    policy: &EffectivePolicy,
) -> Result<AcquisitionMethod, VideoError> {
    match kind {
        VideoSourceKind::LocalFile => Ok(AcquisitionMethod::LocalFile),
        VideoSourceKind::TranscriptOnly => Ok(AcquisitionMethod::None),
        VideoSourceKind::DirectUrl if opts.transcript_path.is_some() && opts.metadata_only => {
            Ok(AcquisitionMethod::None)
        }
        VideoSourceKind::DirectUrl => Ok(AcquisitionMethod::DirectDownload),
        VideoSourceKind::YouTube if opts.transcript_path.is_some() && opts.metadata_only => {
            Ok(AcquisitionMethod::None)
        }
        VideoSourceKind::YouTube if policy.video.allow_external_downloaders => {
            Ok(AcquisitionMethod::ExternalDownloader)
        }
        VideoSourceKind::YouTube if policy.video.allow_browser_automation => {
            Ok(AcquisitionMethod::BrowserAutomation)
        }
        VideoSourceKind::YouTube => Err(VideoError::PolicyDenied {
            reason: "YouTube ingest requires an external downloader or browser automation gate"
                .into(),
        }),
    }
}

fn choose_transcription_source(
    opts: &ResolveOpts,
    policy: &EffectivePolicy,
) -> TranscriptionSource {
    if opts.transcript_path.is_some() || opts.metadata_only {
        TranscriptionSource::UserTranscript
    } else if opts.prefer_caption && policy.video.allow_caption_capture {
        TranscriptionSource::CapturedCaption
    } else {
        TranscriptionSource::LocalAsr
    }
}

fn is_video_file_extension(input: &str) -> bool {
    let lower = input.to_ascii_lowercase();
    [".mp4", ".mkv", ".mov", ".webm", ".m4v"]
        .iter()
        .any(|suffix| lower.ends_with(suffix))
}
