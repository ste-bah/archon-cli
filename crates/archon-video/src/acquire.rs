use std::io::{self, Write};
use std::path::PathBuf;

use archon_policy::{EffectivePolicy, VideoPolicy};
use async_trait::async_trait;
use tokio::process::Command;

use crate::errors::VideoError;
use crate::source::AcquisitionMethod;

#[async_trait]
pub trait AcquisitionAdapter: Send + Sync {
    async fn acquire(&self, url: &str, opts: &AcquireOptions) -> Result<AcquiredMedia, VideoError>;

    fn method(&self) -> AcquisitionMethod;
}

#[derive(Debug, Clone)]
pub struct AcquireOptions {
    pub policy: EffectivePolicy,
    pub audio_only: bool,
    pub yes: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AcquiredMedia {
    pub local_path: PathBuf,
    pub acquisition_method: AcquisitionMethod,
    pub estimated_bytes: Option<u64>,
}

pub struct ExternalDownloaderAdapter {
    pub bin: String,
}

impl ExternalDownloaderAdapter {
    pub fn from_policy(policy: &VideoPolicy) -> Self {
        let bin = std::env::var("ARCHON_YTDLP_BIN")
            .unwrap_or_else(|_| policy.acquire.external_downloader_bin.clone());
        Self { bin }
    }
}

#[async_trait]
impl AcquisitionAdapter for ExternalDownloaderAdapter {
    async fn acquire(&self, url: &str, opts: &AcquireOptions) -> Result<AcquiredMedia, VideoError> {
        let decision = opts.policy.video_acquisition_decision("ExternalDownloader");
        if !decision.allowed {
            return Err(VideoError::PolicyDenied {
                reason: decision.reason,
            });
        }
        confirm_download(url, "yt-dlp", None, opts.yes, &opts.policy.video)?;

        let output_dir = std::env::current_dir()
            .unwrap_or_else(|_| std::env::temp_dir())
            .join(".archon")
            .join("video-artifacts")
            .join("downloads");
        std::fs::create_dir_all(&output_dir).map_err(|e| VideoError::AcquisitionFailed {
            message: format!("create video download directory: {e}"),
        })?;
        let mut cmd = Command::new(&self.bin);
        if opts.audio_only {
            cmd.args(["-x", "--audio-format", "wav"]);
        } else {
            let video_format = visual_video_format_selector();
            cmd.args(["-f", video_format.as_str()]);
        }
        cmd.args(["--no-playlist", "--paths"]);
        cmd.arg(&output_dir);
        cmd.args(["-o", "archon-video-%(id)s.%(ext)s"]);
        cmd.args(["--print", "after_move:filepath"]);
        cmd.arg(url);
        let acquire = &opts.policy.video.acquire;
        if !acquire.browser_profile.is_empty() {
            cmd.args(["--cookies-from-browser", &acquire.browser_profile]);
        }
        if !acquire.po_token_provider.is_empty() {
            cmd.args(["--po-token-server", &acquire.po_token_provider]);
        }
        if let Some(ffmpeg_location) = ffmpeg_location_arg() {
            cmd.args(["--ffmpeg-location", &ffmpeg_location]);
        }
        let output = cmd.output().await.map_err(|_| VideoError::BinaryNotFound {
            name: "yt-dlp".into(),
            path: self.bin.clone(),
        })?;
        if !output.status.success() {
            return Err(platform_or_acquisition_error(&output.stderr));
        }
        let local_path = parse_downloaded_path(&output.stdout)?;
        Ok(AcquiredMedia {
            local_path,
            acquisition_method: AcquisitionMethod::ExternalDownloader,
            estimated_bytes: None,
        })
    }

    fn method(&self) -> AcquisitionMethod {
        AcquisitionMethod::ExternalDownloader
    }
}

pub struct BrowserAutomationAdapter {
    pub bin: String,
}

#[async_trait]
impl AcquisitionAdapter for BrowserAutomationAdapter {
    async fn acquire(&self, url: &str, opts: &AcquireOptions) -> Result<AcquiredMedia, VideoError> {
        let decision = opts.policy.video_acquisition_decision("BrowserAutomation");
        if !decision.allowed {
            return Err(VideoError::PolicyDenied {
                reason: decision.reason,
            });
        }
        confirm_download(
            url,
            "browser automation",
            None,
            opts.yes,
            &opts.policy.video,
        )?;
        Err(VideoError::AcquisitionFailed {
            message: format!(
                "browser automation acquisition is not implemented for binary {} yet",
                self.bin
            ),
        })
    }

    fn method(&self) -> AcquisitionMethod {
        AcquisitionMethod::BrowserAutomation
    }
}

pub struct MockAcquisitionAdapter {
    pub fixture_path: PathBuf,
    pub method: AcquisitionMethod,
}

#[async_trait]
impl AcquisitionAdapter for MockAcquisitionAdapter {
    async fn acquire(
        &self,
        _url: &str,
        _opts: &AcquireOptions,
    ) -> Result<AcquiredMedia, VideoError> {
        Ok(AcquiredMedia {
            local_path: self.fixture_path.clone(),
            acquisition_method: self.method.clone(),
            estimated_bytes: None,
        })
    }

    fn method(&self) -> AcquisitionMethod {
        self.method.clone()
    }
}

pub fn confirm_download(
    url: &str,
    method: &str,
    estimated_bytes: Option<u64>,
    yes: bool,
    policy: &VideoPolicy,
) -> Result<(), VideoError> {
    println!("Archon will fetch: {url}");
    println!("Method: {method}");
    if let Some(bytes) = estimated_bytes {
        println!("Estimated size: {} MB", bytes / 1_048_576);
    }
    if yes && !policy.require_user_confirmation_for_download {
        return Ok(());
    }
    if yes {
        return Ok(());
    }
    print!("Proceed? [y/N] ");
    io::stdout()
        .flush()
        .map_err(|e| acquisition_error(format!("failed to flush confirmation prompt: {e}")))?;
    let mut line = String::new();
    io::stdin()
        .read_line(&mut line)
        .map_err(|e| acquisition_error(format!("failed to read confirmation: {e}")))?;
    if line.trim().eq_ignore_ascii_case("y") {
        Ok(())
    } else {
        Err(acquisition_error("user did not confirm download"))
    }
}

fn platform_or_acquisition_error(stderr: &[u8]) -> VideoError {
    let stderr = String::from_utf8_lossy(stderr).to_string();
    let lower = stderr.to_ascii_lowercase();
    if lower.contains("sign in") || lower.contains("po token") || lower.contains("bot") {
        return acquisition_error(format!(
            "Platform blocked acquisition: {}. Archon does not attempt to bypass platform restrictions. Provide --transcript directly or authenticate your own browser profile.",
            stderr.lines().next().unwrap_or("unknown platform block")
        ));
    }
    acquisition_error(stderr)
}

fn acquisition_error(message: impl Into<String>) -> VideoError {
    VideoError::AcquisitionFailed {
        message: message.into(),
    }
}

fn parse_downloaded_path(stdout: &[u8]) -> Result<PathBuf, VideoError> {
    let stdout = String::from_utf8_lossy(stdout);
    stdout
        .lines()
        .rev()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(PathBuf::from)
        .find(|path| path.exists())
        .ok_or_else(|| {
            acquisition_error(format!(
                "yt-dlp completed but did not report an existing output path; stdout: {stdout}"
            ))
        })
}

fn ffmpeg_location_arg() -> Option<String> {
    if let Ok(path) = std::env::var("ARCHON_FFMPEG_BIN") {
        let path = PathBuf::from(path);
        return Some(
            path.parent()
                .filter(|parent| !parent.as_os_str().is_empty())
                .unwrap_or(path.as_path())
                .display()
                .to_string(),
        );
    }
    let homebrew = PathBuf::from("/opt/homebrew/bin/ffmpeg");
    homebrew.exists().then(|| "/opt/homebrew/bin".to_string())
}

fn visual_video_format_selector() -> String {
    std::env::var("ARCHON_YTDLP_VIDEO_FORMAT")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| "best[height<=720][ext=mp4]/best[height<=720]/best[ext=mp4]/best".into())
}
