use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct VideoPolicy {
    pub enabled: bool,
    pub allow_youtube: bool,
    pub allow_direct_urls: bool,
    pub allow_external_downloaders: bool,
    pub allow_browser_automation: bool,
    pub allow_caption_capture: bool,
    pub allow_cloud_asr: bool,
    pub allow_cloud_vlm: bool,
    pub require_user_confirmation_for_download: bool,
    pub max_duration_minutes: u32,
    pub max_download_mb: u64,
    pub max_frames: u32,
    pub frame_interval_secs: u32,
    pub scene_change_threshold: f32,
    pub dedupe_threshold: f32,
    pub acquire: VideoAcquirePolicy,
    pub asr: VideoAsrPolicy,
    pub summary: VideoSummaryPolicy,
    pub frames: VideoFramesPolicy,
}

impl Default for VideoPolicy {
    fn default() -> Self {
        Self {
            enabled: false,
            allow_youtube: false,
            allow_direct_urls: false,
            allow_external_downloaders: false,
            allow_browser_automation: false,
            allow_caption_capture: false,
            allow_cloud_asr: false,
            allow_cloud_vlm: false,
            require_user_confirmation_for_download: true,
            max_duration_minutes: 120,
            max_download_mb: 2048,
            max_frames: 500,
            frame_interval_secs: 10,
            scene_change_threshold: 0.35,
            dedupe_threshold: 0.94,
            acquire: VideoAcquirePolicy::default(),
            asr: VideoAsrPolicy::default(),
            summary: VideoSummaryPolicy::default(),
            frames: VideoFramesPolicy::default(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct VideoAcquirePolicy {
    pub browser_profile: String,
    pub external_downloader_bin: String,
    pub po_token_provider: String,
}

impl Default for VideoAcquirePolicy {
    fn default() -> Self {
        Self {
            browser_profile: "default".into(),
            external_downloader_bin: "yt-dlp".into(),
            po_token_provider: String::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct VideoAsrPolicy {
    pub provider: String,
    pub model: String,
    pub device: String,
    pub vad_stable_timestamps: bool,
    pub model_cache_dir: String,
    pub model_source: String,
    pub diarization: bool,
}

impl Default for VideoAsrPolicy {
    fn default() -> Self {
        Self {
            provider: "whisper-rs".into(),
            model: "base".into(),
            device: "auto".into(),
            vad_stable_timestamps: false,
            model_cache_dir: String::new(),
            model_source: String::new(),
            diarization: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct VideoSummaryPolicy {
    pub enabled: bool,
    pub allow_llm_summary: bool,
    pub allow_cloud_summary: bool,
    pub provider: String,
}

impl Default for VideoSummaryPolicy {
    fn default() -> Self {
        Self {
            enabled: false,
            allow_llm_summary: false,
            allow_cloud_summary: false,
            provider: "disabled".into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct VideoFramesPolicy {
    pub mode: String,
    pub ocr: bool,
    pub vlm: bool,
}

impl Default for VideoFramesPolicy {
    fn default() -> Self {
        Self {
            mode: "scene".into(),
            ocr: true,
            vlm: true,
        }
    }
}

#[derive(Debug, Default, Deserialize)]
pub(crate) struct RawVideoPolicy {
    enabled: Option<bool>,
    allow_youtube: Option<bool>,
    allow_direct_urls: Option<bool>,
    allow_external_downloaders: Option<bool>,
    allow_browser_automation: Option<bool>,
    allow_caption_capture: Option<bool>,
    allow_cloud_asr: Option<bool>,
    allow_cloud_vlm: Option<bool>,
    require_user_confirmation_for_download: Option<bool>,
    max_duration_minutes: Option<u32>,
    max_download_mb: Option<u64>,
    max_frames: Option<u32>,
    frame_interval_secs: Option<u32>,
    scene_change_threshold: Option<f32>,
    dedupe_threshold: Option<f32>,
    acquire: Option<RawVideoAcquirePolicy>,
    asr: Option<RawVideoAsrPolicy>,
    summary: Option<RawVideoSummaryPolicy>,
    frames: Option<RawVideoFramesPolicy>,
}

#[derive(Debug, Default, Deserialize)]
struct RawVideoAcquirePolicy {
    browser_profile: Option<String>,
    external_downloader_bin: Option<String>,
    po_token_provider: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
struct RawVideoAsrPolicy {
    provider: Option<String>,
    model: Option<String>,
    device: Option<String>,
    vad_stable_timestamps: Option<bool>,
    model_cache_dir: Option<String>,
    model_source: Option<String>,
    diarization: Option<bool>,
}

#[derive(Debug, Default, Deserialize)]
struct RawVideoSummaryPolicy {
    enabled: Option<bool>,
    allow_llm_summary: Option<bool>,
    allow_cloud_summary: Option<bool>,
    provider: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
struct RawVideoFramesPolicy {
    mode: Option<String>,
    ocr: Option<bool>,
    vlm: Option<bool>,
}

pub(crate) fn apply_video(policy: &mut VideoPolicy, raw: RawVideoPolicy) {
    apply_bool(&mut policy.enabled, raw.enabled);
    apply_bool(&mut policy.allow_youtube, raw.allow_youtube);
    apply_bool(&mut policy.allow_direct_urls, raw.allow_direct_urls);
    apply_bool(
        &mut policy.allow_external_downloaders,
        raw.allow_external_downloaders,
    );
    apply_bool(
        &mut policy.allow_browser_automation,
        raw.allow_browser_automation,
    );
    apply_bool(&mut policy.allow_caption_capture, raw.allow_caption_capture);
    apply_bool(&mut policy.allow_cloud_asr, raw.allow_cloud_asr);
    apply_bool(&mut policy.allow_cloud_vlm, raw.allow_cloud_vlm);
    apply_bool(
        &mut policy.require_user_confirmation_for_download,
        raw.require_user_confirmation_for_download,
    );
    apply_copy(&mut policy.max_duration_minutes, raw.max_duration_minutes);
    apply_copy(&mut policy.max_download_mb, raw.max_download_mb);
    apply_copy(&mut policy.max_frames, raw.max_frames);
    apply_copy(&mut policy.frame_interval_secs, raw.frame_interval_secs);
    apply_copy(
        &mut policy.scene_change_threshold,
        raw.scene_change_threshold,
    );
    apply_copy(&mut policy.dedupe_threshold, raw.dedupe_threshold);

    if let Some(acquire) = raw.acquire {
        apply_acquire(&mut policy.acquire, acquire);
    }
    if let Some(asr) = raw.asr {
        apply_asr(&mut policy.asr, asr);
    }
    if let Some(summary) = raw.summary {
        apply_summary(&mut policy.summary, summary);
    }
    if let Some(frames) = raw.frames {
        apply_frames(&mut policy.frames, frames);
    }
}

fn apply_acquire(policy: &mut VideoAcquirePolicy, raw: RawVideoAcquirePolicy) {
    apply_string(&mut policy.browser_profile, raw.browser_profile);
    apply_string(
        &mut policy.external_downloader_bin,
        raw.external_downloader_bin,
    );
    apply_string(&mut policy.po_token_provider, raw.po_token_provider);
}

fn apply_asr(policy: &mut VideoAsrPolicy, raw: RawVideoAsrPolicy) {
    apply_string(&mut policy.provider, raw.provider);
    apply_string(&mut policy.model, raw.model);
    apply_string(&mut policy.device, raw.device);
    apply_bool(&mut policy.vad_stable_timestamps, raw.vad_stable_timestamps);
    apply_string(&mut policy.model_cache_dir, raw.model_cache_dir);
    apply_string(&mut policy.model_source, raw.model_source);
    apply_bool(&mut policy.diarization, raw.diarization);
}

fn apply_summary(policy: &mut VideoSummaryPolicy, raw: RawVideoSummaryPolicy) {
    apply_bool(&mut policy.enabled, raw.enabled);
    apply_bool(&mut policy.allow_llm_summary, raw.allow_llm_summary);
    apply_bool(&mut policy.allow_cloud_summary, raw.allow_cloud_summary);
    apply_string(&mut policy.provider, raw.provider);
}

fn apply_frames(policy: &mut VideoFramesPolicy, raw: RawVideoFramesPolicy) {
    apply_string(&mut policy.mode, raw.mode);
    apply_bool(&mut policy.ocr, raw.ocr);
    apply_bool(&mut policy.vlm, raw.vlm);
}

fn apply_bool(target: &mut bool, value: Option<bool>) {
    if let Some(value) = value {
        *target = value;
    }
}

fn apply_copy<T: Copy>(target: &mut T, value: Option<T>) {
    if let Some(value) = value {
        *target = value;
    }
}

fn apply_string(target: &mut String, value: Option<String>) {
    if let Some(value) = value {
        *target = value;
    }
}
