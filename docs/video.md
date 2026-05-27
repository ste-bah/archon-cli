# Video Evidence Ingest

Archon can ingest video evidence into the same document, provenance, search, and
knowledge-base stack used for PDFs, images, Markdown, and text. Video transcript,
OCR, VLM, and summary output is stored as ordinary `doc_chunks`, with
`video_chunk_timeref` rows preserving timestamp provenance for `video@MM:SS`
citations.

For a real-world end-to-end walkthrough, see
[YouTube video evidence with local Whisper](cookbook/video-evidence-youtube-whisper.md).

## Quick Start

```bash
archon video ingest ./lecture.mp4 --transcript ./lecture.vtt --frames none
archon video inspect <video-id>
archon video transcript <video-id> --format vtt
archon docs answer "what claim did the speaker make?"
```

Inside the TUI, use the mirrored slash form:

```text
/video ingest ./lecture.mp4 --transcript ./lecture.vtt --frames none
/video ingest "https://youtu.be/abc123" --frames hybrid --asr whisper-cpp --yes
/video ingest "https://youtu.be/abc123" --kb trading-elliott-wave --frames hybrid --asr whisper-cpp --yes
/video inspect <video-id>
```

The TUI slash command is a CLI mirror: flags such as `--frames`, `--asr`,
`--metadata-only`, `--vlm`, `--kb`, and `--yes` are passed through to the same
handler as `archon video ingest`. In a shell, quote video URLs that contain `&`
or other shell metacharacters; inside the TUI, quote URLs with spaces only.

## Transcript-Only Workflow

Use transcript-only mode when you already have `.vtt`, `.srt`, `.ttml`, JSON, or
timestamped text. The media file does not need to be downloaded.

```bash
archon video ingest "https://www.youtube.com/watch?v=abc123" \
  --transcript ./talk.vtt \
  --metadata-only
```

Transcript cues become `video_transcript` chunks and are immediately visible to
`archon docs search`, `archon docs answer`, and `archon kb process`.

## Local Video With ASR

Local ASR is policy-gated and provider-selected. The v1 surface includes
subprocess adapters for `whisper-cpp`/`faster-whisper` and a managed model-cache
resolver; `whisper-rs` currently records the requested backend and reports a
structured unavailable error unless compiled into a future build path.

```bash
archon video ingest ./interview.mp4 --asr whisper-cpp --frames scene --yes
```

`ARCHON_FFMPEG_BIN`, `ARCHON_FFPROBE_BIN`, `ARCHON_WHISPER_BIN`, and
`ARCHON_FASTER_WHISPER_BIN` can point Archon at non-default binaries. On
Apple Silicon with Homebrew installs, the common overrides are:

```bash
ARCHON_FFMPEG_BIN=/opt/homebrew/bin/ffmpeg \
ARCHON_WHISPER_BIN=/opt/homebrew/bin/whisper-cli \
archon video ingest ./interview.mp4 --asr whisper-cpp --frames scene --yes
```

`whisper-cpp` runs `whisper-cli` with JSON output enabled, then stores each
transcript segment as timecoded document evidence. Archon extracts a temporary
16 kHz mono WAV with `ffmpeg` before calling the ASR binary.

## YouTube With User Transcript

YouTube URLs with a user transcript and `--metadata-only` do not require
downloaders, browser automation, or caption capture. Archon stores the URL and
policy snapshot, then ingests the transcript as evidence.

```bash
archon video ingest "https://youtu.be/abc123" --transcript ./talk.srt --metadata-only
```

Playlist and channel URLs are rejected. Use a single video URL.

## YouTube With Local ASR

For full YouTube media ingest, enable the external-downloader policy path and
set `external_downloader_bin` or `ARCHON_YTDLP_BIN`. Archon calls `yt-dlp`,
writes the downloaded file under `<workspace>/.archon/video-artifacts/downloads`,
then runs the selected ASR/frame paths against that local file.

```bash
archon video ingest "https://youtu.be/abc123" --frames hybrid --asr whisper-cpp --yes
```

`yt-dlp` may need `ffmpeg` for format merging or audio extraction. Current
Archon builds pass the Homebrew `ffmpeg` directory to `yt-dlp` automatically
when `/opt/homebrew/bin/ffmpeg` exists, or when `ARCHON_FFMPEG_BIN` points to an
explicit binary. For video+frame ingest, Archon asks `yt-dlp` for a
frame-friendly MP4-oriented format by default:
`best[height<=720][ext=mp4]/best[height<=720]/best[ext=mp4]/best`. Override
that with `ARCHON_YTDLP_VIDEO_FORMAT` when you need a different local policy.

If `[policy.video].allow_caption_capture = true`, Archon first tries to capture
English VTT captions with `yt-dlp`. Captions become timecoded transcript chunks.
If no usable captions exist, it falls back to the configured ASR provider.

To add the YouTube evidence to an existing KB, include `--kb <name>` on ingest
and then process or search that bucket:

```bash
archon video ingest "https://youtu.be/abc123" --kb trading-elliott-wave --frames hybrid --asr whisper-cpp --yes
archon kb process --kb trading-elliott-wave --claims --entities --relations
archon kb search --kb trading-elliott-wave "wave count invalidation"
```

## Policy-Gated Acquisition

External acquisition is default-deny. If enabled, Archon can call configured
downloaders such as `yt-dlp`; platform blocks are reported honestly and Archon
does not attempt proxy rotation, CAPTCHA bypass, fingerprint spoofing, or other
anti-evasion behaviour.

```toml
[policy.video]
enabled = true
allow_youtube = true
allow_external_downloaders = true
require_user_confirmation_for_download = true

[policy.video.acquire]
external_downloader_bin = "/opt/homebrew/bin/yt-dlp"
browser_profile = "default"
po_token_provider = ""

[policy.video.asr]
provider = "whisper-cpp"
model = "/Users/you/Library/Application Support/archon/models/whisper/ggml-small.en.bin"
device = "auto"
```

## Chart And Diagram Extraction

Frame extraction uses `ffmpeg` in `interval`, `scene`, or `hybrid` mode. If
`ffmpeg` cannot write frames from a difficult container or codec, Archon can
fall back to a local Python/OpenCV sampler when `opencv-python` is available.
Frames are deduplicated with a perceptual hash before OCR/VLM evidence chunks
are written.

```bash
archon video ingest ./market-review.mp4 --frames hybrid --vlm --yes
archon video frames <video-id>
archon video reprocess <video-id> --frames
```

VLM frame descriptions use the dedicated video-frame prompt, which asks the
provider to identify chart axes, units, legends, slide bullets, diagrams,
visible trends, and uncertainty without inferring beyond visible evidence.

## Provider Setup

OCR reuses the existing document OCR provider. Frame VLM reuses the existing
single active docs VLM provider. Video ASR uses the `[policy.video.asr]` policy
block plus local binaries.

```toml
[policy.workers]
ocr = "allow-local"
vlm = "allow-local"

[policy.docs.vlm]
enabled = true
mode = "local"
provider = "ollama"
allow_cloud = false
```

Cloud VLM/summary providers require the matching cloud policy gates.

Local image OCR defaults to Tesseract. Set `ARCHON_OCR_ENGINE=rapidocr` to try
RapidOCR first, or leave it unset to use RapidOCR only as a fallback if
Tesseract fails. RapidOCR requires Python plus `rapidocr_onnxruntime` or
`rapidocr`; OpenCV frame fallback requires `opencv-python`.

## Policy Configuration Reference

Video ingest is default-deny. Enable only the paths you intend to use:

```toml
[policy.video]
enabled = true
allow_youtube = false
allow_direct_urls = false
allow_external_downloaders = false
allow_browser_automation = false
allow_caption_capture = false
allow_cloud_asr = false
allow_cloud_vlm = false
require_user_confirmation_for_download = true
max_duration_minutes = 120
max_download_mb = 2048
max_frames = 500
frame_interval_secs = 10
scene_change_threshold = 0.35
dedupe_threshold = 0.94

[policy.video.asr]
provider = "whisper-rs"
model = "base"
device = "auto"
vad_stable_timestamps = false
model_cache_dir = ""
model_source = ""
diarization = false

[policy.video.frames]
mode = "scene"
ocr = true
vlm = true

[policy.video.summary]
enabled = false
allow_llm_summary = false
allow_cloud_summary = false
provider = "disabled"
```

Project-local policy lives at `<workspace>/.archon/policy.toml`; user policy can
live at `~/.archon/policy.toml`. Later files override earlier files.

## Storage And Provenance

Video ingest creates:

- `video_sources`, `video_tracks`, `video_transcript_segments`,
  `video_frame_descriptions`, and `video_chunk_timeref`
- ordinary `doc_artifacts`, `doc_chunks`, and `doc_provenance_edges`
- optional `video_summary` chunks when summary policy is enabled

Because video evidence is stored as ordinary document chunks, `archon kb process`
and document retrieval consume it without a video-specific flag.

## Compliance And TOS Notice

Only ingest video you are allowed to process. For platform-hosted video, respect
the platform's terms, account permissions, and local law. Archon does not include
proxy rotation, bulk harvesting, CAPTCHA bypass, fingerprint spoofing, or hidden
anti-bot evasion.

## Troubleshooting

- `NoEvidenceExtracted`: provide a transcript, enable a working ASR provider, or
  use frame extraction with OCR/VLM.
- `binary not found`: set `ARCHON_FFMPEG_BIN`, `ARCHON_FFPROBE_BIN`, or the
  relevant ASR binary env var.
- YouTube ingest remains `running` with `0` tracks/chunks: inspect active
  processes for `yt-dlp`, `ffmpeg`, or `whisper-cli`. If `ffmpeg` is idle and a
  temporary `archon-video-audio-*.wav` file is `0B`, use a build that runs
  `ffmpeg` non-interactively; current builds pass `-nostdin -y` for audio
  extraction.
- Transcript ingest succeeds but frame extraction writes `0` frames: current
  builds extract PNG frames instead of MJPEG/JPEG to avoid AV1/non-full-range
  YUV encoder failures. After updating the binary, run
  `archon video reprocess <video-id> --frames`.
- `yt-dlp` cannot find `ffmpeg`: set `ARCHON_FFMPEG_BIN` before starting the
  shell/TUI, or install Homebrew `ffmpeg` at `/opt/homebrew/bin/ffmpeg`.
- `yt-dlp` warns about no supported JavaScript runtime: URL extraction may still
  work, but some YouTube formats can be unavailable. Install a supported runtime
  such as Deno or Node if `yt-dlp` reports that formats are missing.
- `video@MM:SS` citations missing: confirm `video_chunk_timeref` rows exist via
  `archon video inspect <video-id>`.
- No summary: `[policy.video.summary]` is disabled by default.
