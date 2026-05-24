# Environment variables

| Variable | Description |
|---|---|
| `ANTHROPIC_API_KEY` | Claude API key (unless using OAuth) |
| `ANTHROPIC_BASE_URL` | Override Anthropic-format endpoint or base URL (LiteLLM, Ollama, DeepSeek Anthropic API, etc.) |
| `ANTHROPIC_MODEL` | Override the main Anthropic-format session model, useful for providers such as DeepSeek that document Claude Code-style env setup |
| `ARCHON_API_KEY` | Alias for `ANTHROPIC_API_KEY` |
| `ARCHON_OAUTH_TOKEN` | Pre-set OAuth bearer token (skips login) |
| `ANTHROPIC_AUTH_TOKEN` | Legacy bearer token alias |
| `OPENAI_API_KEY` | OpenAI API key for embeddings, LLM provider, and STT |
| `GOOGLE_API_KEY` | Google Generative Language API key for Gemini VLM image descriptions |
| `ARCHON_MEMORY_OPENAIKEY` | Alias for `OPENAI_API_KEY` (memory embeddings only) |
| `ARCHON_CODEX_DISABLED` | Disable Codex provider resolution when set to `1`, `true`, or `yes` |
| `ARCHON_CODEX_BASE_URL` | Override Codex backend URL for local mocks or diagnostics |
| `ARCHON_CODEX_APP_SERVER_URL` | Override configured Codex app-server WebSocket endpoint for local diagnostics |
| `ARCHON_CODEX_ORIGINATOR` | Override Codex spoof `originator` field |
| `ARCHON_CODEX_USER_AGENT` | Override Codex spoof user agent, subject to anti-impersonation validation |
| `ARCHON_CODEX_CLIENT_ID` | Override Codex OAuth client id (`app_...`) |
| `ARCHON_CODEX_BETA` | Override Codex `OpenAI-Beta` header |
| `ARCHON_CODEX_FETCH_URL` | Reserved Codex manifest fetch override |
| `ARCHON_CODEX_SPOOF_ALLOW_MIXED` | Dev-only Codex spoof-source mixing escape hatch |
| `ARCHON_CODEX_E2E` | Enables opt-in real-backend Codex tests; never use in scheduled CI |
| `ARCHON_CODEX_SMOKE_PROMPT` | Manual Codex smoke prompt override |
| `ARCHON_CODEX_SMOKE_EXPECTED` | Manual Codex smoke expected marker |
| `ARCHON_CODEX_SMOKE_MODEL` | Manual Codex smoke model override |
| `ARCHON_CONFIG` | Override config file path |
| `ARCHON_LOG` | Override log level |
| `RUST_LOG` | Tracing subscriber filter |
| `ARCHON_DATA_DIR` | Override per-user state dir (default: platform data dir + `archon`) |
| `ARCHON_EVIDENCE_DB_PATH` | Override the shared project evidence store; otherwise evidence surfaces use `<workspace>/.archon/archon-data.db` |
| `ARCHON_COMPLETION_DB_PATH` | Override completion evidence store path only; otherwise `ARCHON_EVIDENCE_DB_PATH` or the shared project evidence store is used |
| `ARCHON_DOCS_DB_PATH` | Override docs evidence store path only; otherwise `ARCHON_EVIDENCE_DB_PATH` or the shared project evidence store is used |
| `ARCHON_LEARNING_DB_PATH` | Override governed/pipeline-learning evidence store path only; otherwise `ARCHON_EVIDENCE_DB_PATH` or the shared project evidence store is used. Setting this disables automatic migration from legacy `<workspace>/.archon/learning.db` |
| `ARCHON_SESSION_DB_PATH` | Override session database path; otherwise `[session].db_path`, then platform data dir + `archon/sessions/sessions.db` |
| `ARCHON_SESSIONS_DIR` | Override session directory |
| `ARCHON_NO_TUI` | Force headless mode |
| `ARCHON_TRUST_USER_GRAMMARS` | Set to `1`, `true`, or `yes` to allow TUI syntax highlighting to load user-provided tree-sitter `.so` grammars |
| `ARCHON_FFMPEG_BIN` | Override the `ffmpeg` binary used by video frame/audio extraction |
| `ARCHON_FFPROBE_BIN` | Override the `ffprobe` binary used by video metadata extraction |
| `ARCHON_WHISPER_BIN` | Override the `whisper-cli` binary used by `whisper-cpp` video ASR |
| `ARCHON_FASTER_WHISPER_BIN` | Override the `faster-whisper` binary used by video ASR |
| `ARCHON_YTDLP_BIN` | Override the `yt-dlp` binary used by YouTube/video acquisition |
| `ARCHON_YTDLP_VIDEO_FORMAT` | Override the MP4-oriented `yt-dlp` format selector used for video+frame ingest |
| `ARCHON_PDFTOTEXT_BIN` | Override the `pdftotext` binary used by document/PDF extraction |
| `ARCHON_PDFIMAGES_BIN` | Override the `pdfimages` binary used by embedded PDF image extraction |
| `ARCHON_PDFTOPPM_BIN` | Override the `pdftoppm` binary used by rendered PDF page fallback |
| `ARCHON_TESSERACT_BIN` | Override the `tesseract` binary used by local OCR |
| `ARCHON_OCR_ENGINE` | Set to `rapidocr` to prefer RapidOCR for image/frame OCR, or `tesseract` to disable RapidOCR fallback |
| `ARCHON_RAPIDOCR_PYTHON` | Override the Python binary used for optional RapidOCR image/frame OCR |
| `ARCHON_RAPIDOCR_MIN_SCORE` | Minimum RapidOCR confidence score, default `0.55` |
| `ARCHON_VIDEO_FRAME_FALLBACK` | Set to `0`, `false`, `no`, or `off` to disable Python/OpenCV frame fallback |
| `ARCHON_VIDEO_OPENCV_PYTHON` | Override the Python binary used for optional OpenCV frame fallback |
| `EDITOR` | Used by `/commit` and skill workflows that open an editor |
| `SHELL` | Inherited by `Bash` tool subprocesses |
| `HOME` | Used to resolve `~/.config/archon/` and `~/.local/share/archon/` |
| `XDG_CONFIG_HOME` | Linux/macOS: overrides `~/.config` base |
| `XDG_DATA_HOME` | Linux/macOS: overrides `~/.local/share` base |
| `APPDATA` | Windows: per-user state base |
| `SSH_AUTH_SOCK` | Used by `archon remote ssh` for agent forwarding |

Video binary overrides must be exported before starting the TUI if you want
slash commands such as `/video ingest ... --asr whisper-cpp` to inherit them.
For Apple Silicon Homebrew installs, common values are
`ARCHON_FFMPEG_BIN=/opt/homebrew/bin/ffmpeg`,
`ARCHON_FFPROBE_BIN=/opt/homebrew/bin/ffprobe`,
`ARCHON_WHISPER_BIN=/opt/homebrew/bin/whisper-cli`, and
`ARCHON_YTDLP_BIN=/opt/homebrew/bin/yt-dlp`.

## Resolution order for credentials

1. `~/.archon/.credentials.json` (from `archon auth login --provider anthropic`)
2. `~/.claude/.credentials.json` (deprecated fallback when the Archon file is absent)
3. `ARCHON_OAUTH_TOKEN` env
4. `ANTHROPIC_AUTH_TOKEN` env (legacy)
5. `ANTHROPIC_API_KEY` env
6. `ARCHON_API_KEY` env (alias)

## Resolution order for OpenAI key

1. `OPENAI_API_KEY` env (all features)
2. `ARCHON_MEMORY_OPENAIKEY` env (memory embeddings only)
3. `[llm.openai] api_key` in config

If none are set, archon uses local fastembed for embeddings (no network calls) and disables OpenAI-dependent features.

## Resolution order for Gemini VLM key

1. The env var named by `[policy.docs.vlm.gemini] api_key_env` (default: `GOOGLE_API_KEY`)
2. `googleApiKey` in `~/.archon/.credentials.json`, written by `archon auth login --provider google`

Gemini is only used when `[policy.docs.vlm] provider = "gemini"` and both cloud VLM gates allow it.

## Codex OAuth and provider parity

Codex subscription credentials are stored in `~/.archon/.credentials.json` under
`openaiCodexOauth` after:

```bash
archon auth login --provider openai-codex
```

Set `[llm].provider = "openai-codex"` in config to make the TUI, tool use,
subagents, `/btw`, team runs, coding/research pipelines, and gametheory use the
Codex provider instead of Anthropic. The `ARCHON_CODEX_*` variables only affect
the Codex provider; Anthropic OAuth/API-key/proxy settings remain separate.
Never print access or refresh tokens in transcripts.

Codex-backed sessions normalize inherited Claude-shaped `[api].default_model`
values before provider calls. Sonnet/Opus-tier defaults map through
`[models.openai-codex].default`, Haiku-tier defaults map through
`[models.openai-codex].mini`, and concrete Codex model ids are preserved.

## DeepSeek Anthropic API

For full TUI, subagent, and pipeline compatibility with DeepSeek's
Anthropic-compatible agent endpoint, keep `[llm].provider = "anthropic"` and
set:

```bash
export ANTHROPIC_AUTH_TOKEN="<your DeepSeek API key>"
export ANTHROPIC_BASE_URL="https://api.deepseek.com/anthropic"
export ANTHROPIC_MODEL="deepseek-v4-pro[1m]"
```

`ANTHROPIC_BASE_URL` may be either a full `/v1/messages` endpoint or a provider
base URL; base URLs are expanded to `/v1/messages` internally.

## Logging filters

`RUST_LOG` accepts standard `tracing` filter syntax:

```bash
RUST_LOG=archon=trace archon                       # All archon crates trace
RUST_LOG=archon_pipeline=debug,archon_llm=trace archon
RUST_LOG=info,archon_memory::garden=debug archon   # Default info, garden debug
```

`ARCHON_LOG` is a simpler shorthand:

```bash
ARCHON_LOG=debug archon
ARCHON_LOG=trace archon
```

## See also

- [CLI flags](cli-flags.md)
- [Configuration](config.md)
- [Codex environment variables](../env-vars-codex.md)
- [Authentication setup](../getting-started/installation.md)
