use std::path::PathBuf;

use clap::Subcommand;

#[derive(Subcommand, Debug, Clone)]
pub enum VideoAction {
    /// Ingest a video source (local file, URL, or YouTube)
    Ingest {
        /// Local path, direct URL, or YouTube URL
        source: String,
        /// User-provided transcript file (.vtt, .srt, .ttml, .json, .txt)
        #[arg(long)]
        transcript: Option<PathBuf>,
        /// Frame mode: none, interval, scene, or hybrid
        #[arg(long)]
        frames: Option<String>,
        /// ASR provider: whisper-rs, whisper-cpp, faster-whisper, or disabled
        #[arg(long)]
        asr: Option<String>,
        /// Enable frame VLM descriptions
        #[arg(long)]
        vlm: bool,
        /// Knowledge-base name to attach the video evidence to
        #[arg(long)]
        kb: Option<String>,
        /// Register metadata and provided transcript without media acquisition
        #[arg(long)]
        metadata_only: bool,
        /// Skip interactive confirmation prompts
        #[arg(long, short)]
        yes: bool,
    },
    /// Show status of all video sources
    Status,
    /// Inspect a specific video source
    Inspect { video_id: String },
    /// List frame artifacts for a video
    Frames { video_id: String },
    /// Export transcript in txt, srt, or vtt
    Transcript {
        video_id: String,
        #[arg(long, default_value = "txt")]
        format: String,
    },
    /// Show or rerun the LLM summary
    Summary { video_id: String },
    /// Delete a video source and its document registry rows
    Delete {
        video_id: String,
        /// Confirm destructive deletion
        #[arg(long, short)]
        yes: bool,
    },
    /// Reprocess specific tracks
    Reprocess {
        video_id: String,
        #[arg(long)]
        transcript: bool,
        #[arg(long)]
        frames: bool,
        #[arg(long)]
        ocr: bool,
        #[arg(long)]
        vlm: bool,
        #[arg(long)]
        asr: bool,
        #[arg(long)]
        summary: bool,
    },
    /// List all ingested video sources
    List,
}
