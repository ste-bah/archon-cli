# Video Evidence Ingest

Archon can ingest video evidence into the same document, provenance, search, and
knowledge-base stack used for PDFs, images, Markdown, and text. Video transcript,
OCR, VLM, and summary output is stored as ordinary `doc_chunks`, with
`video_chunk_timeref` rows preserving timestamp provenance for `video@MM:SS`
citations.

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
/video inspect <video-id>
```

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
`ARCHON_FASTER_WHISPER_BIN` can point Archon at non-default binaries.

## YouTube With User Transcript

YouTube URLs with a user transcript and `--metadata-only` do not require
downloaders, browser automation, or caption capture. Archon stores the URL and
policy snapshot, then ingests the transcript as evidence.

```bash
archon video ingest "https://youtu.be/abc123" --transcript ./talk.srt --metadata-only
```

Playlist and channel URLs are rejected. Use a single video URL.

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
external_downloader_bin = "yt-dlp"
browser_profile = "default"
po_token_provider = ""
```

## Chart And Diagram Extraction

Frame extraction uses `ffmpeg` in `interval`, `scene`, or `hybrid` mode. Frames
are deduplicated with a perceptual hash before OCR/VLM evidence chunks are
written.

```bash
archon video ingest ./market-review.mp4 --frames hybrid --vlm --yes
archon video frames <video-id>
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
- `video@MM:SS` citations missing: confirm `video_chunk_timeref` rows exist via
  `archon video inspect <video-id>`.
- No summary: `[policy.video.summary]` is disabled by default.
