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
| `ARCHON_MEMORY_OPENAIKEY` | Alias for `OPENAI_API_KEY` for memory and docs embeddings |
| `ARCHON_MEMORY_EMBEDDING_BASE_URL` | OpenAI-compatible `/v1` base URL for memory embeddings; falls back to `OPENAI_BASE_URL`, then `[memory] embedding_base_url` in config |
| `ARCHON_MEMORY_EMBEDDING_MODEL` | Memory embedding model for OpenAI-compatible providers; falls back to `[memory] embedding_model`, then `text-embedding-3-small` |
| `ARCHON_DOCS_EMBEDDING_PROVIDER` | Docs semantic indexing provider: `auto`, `local`, `openai`, or `disabled` |
| `ARCHON_DOCS_OPENAIKEY` | OpenAI-compatible key used only for docs embeddings |
| `ARCHON_DOCS_EMBEDDING_BASE_URL` | OpenAI-compatible `/v1` base URL for docs embeddings |
| `ARCHON_DOCS_EMBEDDING_MODEL` | Docs embedding model, default `text-embedding-3-small` for OpenAI-compatible providers |
| `ARCHON_DOCS_EMBEDDING_TIMEOUT_SECS` | Per-request timeout for docs OpenAI-compatible embeddings |
| `ARCHON_DOCS_EMBEDDING_LOAD_TIMEOUT_SECS` | Local fastembed model-load timeout before docs indexing fails fast |
| `ARCHON_DOCS_FASTEMBED_INSTANCES` | Number of local fastembed model instances for true local parallel indexing; default follows requested docs index workers and is capped at `4` |
| `ARCHON_DOCS_INDEX_EMBEDDING_WORKERS` | Number of in-process embedding worker batches for `docs index`; default is provider-aware (`1` local fastembed, `2` OpenAI-compatible) |
| `ARCHON_DOCS_INDEX_MAX_IN_FLIGHT_BATCHES` | Backpressure limit for concurrent embedding batches; defaults to the effective worker count |
| `ARCHON_DOCS_INDEX_WRITER_BATCH_SIZE` | Maximum vector rows per single-writer RocksDB/Cozo status flush; default `256` |
| `ARCHON_DOCS_HYBRID_ALWAYS_SEMANTIC` | Force hybrid docs search/answer to run semantic retrieval even when sanitized exact/FTS evidence is already strong; useful for diagnostics |
| `ARCHON_DOC_VECTOR_STORE_DIR` | Override the RocksDB raw-vector store path; default is `<workspace>/.archon/doc-vector-store` |
| `ARCHON_DOCS_LEGACY_COZO_VECTOR_WRITE` | Opt-in compatibility flag to also write new docs embeddings to legacy Cozo `vec_text_chunks` |
| `ARCHON_DOCS_ADAPTIVE_BATCHING` | Set to `0` or `false` to disable adaptive embedding batch sizing during `docs index`; enabled by default |
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
| `ARCHON_CONFIG_DIR` | Override the config directory searched for `config.toml` |
| `ARCHON_MODEL` | Override the session model; lower precedence than the `--model` flag |
| `ARCHON_LOG` | Override log level |
| `ARCHON_LOG_DIR` | Override the per-session log directory; otherwise the platform default log dir is used |
| `RUST_LOG` | Tracing subscriber filter |
| `ARCHON_DATA_DIR` | Override per-user state dir (default: platform data dir + `archon`) |
| `ARCHON_EVIDENCE_DB_PATH` | Override the shared project evidence store; otherwise evidence surfaces use `<workspace>/.archon/archon-data.db` |
| `ARCHON_COMPLETION_DB_PATH` | Override completion evidence store path only; otherwise `ARCHON_EVIDENCE_DB_PATH` or the shared project evidence store is used |
| `ARCHON_DOCS_DB_PATH` | Override docs evidence store path only; otherwise `ARCHON_EVIDENCE_DB_PATH` or the shared project evidence store is used |
| `ARCHON_LEARNING_DB_PATH` | Override governed/pipeline-learning store path only; otherwise learning telemetry uses `<workspace>/.archon/learning-state.db` so idle TUI sessions do not pin the shared docs/video evidence DB |
| `ARCHON_SESSION_DB_PATH` | Override session database path; otherwise `[session].db_path`, then platform data dir + `archon/sessions/sessions.db` |
| `ARCHON_GAMETHEORY_DB_PATH` | Override game-theory evidence store path only; otherwise `ARCHON_EVIDENCE_DB_PATH` or the shared project evidence store is used |
| `ARCHON_LLM_REPLAY` | `record` to write every LLM exchange to a cassette, `replay` to serve from cassettes and never reach the network. Unset means off. Any other value refuses to call a provider at all rather than guess |
| `ARCHON_LLM_CASSETTES` | Where cassettes are read from and written to; default `.archon/cassettes` under the working directory |
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
| `ARCHON_OCR_TIMEOUT_SECS` | Per-image/page OCR timeout before the external binary is killed, default `120` |
| `ARCHON_PDF_RENDER_TIMEOUT_SECS` | Timeout for a single PDF page render, default `1800` |
| `ARCHON_PDF_IMAGE_TIMEOUT_SECS` | Timeout for a single PDF image-enrichment call, default `600` |
| `ARCHON_VIDEO_FRAME_FALLBACK` | Set to `0`, `false`, `no`, or `off` to disable Python/OpenCV frame fallback |
| `ARCHON_VIDEO_OPENCV_PYTHON` | Override the Python binary used for optional OpenCV frame fallback |
| `ARCHON_EVIDENCE_TOOL_BIN` | Override the `archon` binary invoked by the evidence CLI tool |

## TUI

| Variable | Description |
|---|---|
| `ARCHON_THEME_PREFER` | Force the TUI theme; set to `light` for the light theme, otherwise the dark theme is used |
| `ARCHON_TUI_MOUSE_CAPTURE` | Force TUI mouse capture on (`1`, `true`, `on`, `yes`) or off (`0`, `false`, `off`, `no`); when unset, capture defaults to on only under WSL |

## Runtime and agent lifecycle

| Variable | Description |
|---|---|
| `ARCHON_AUTO_BACKGROUND_TASKS` | Set to `1` or `true` to auto-convert long-running subagent tasks into background agents so the parent stops waiting synchronously |
| `ARCHON_FORK_SUBAGENT` | Set to `1` or `true` to enable fork subagent mode |
| `ARCHON_SCRIPT_LIFECYCLE` | Set to `0` or `false` to fall back to the decomposed workflow lifecycle; the scripted (v3) lifecycle is the default when unset |
| `ARCHON_ORCHESTRATED_LIFECYCLE` | Set to `1` or `true` to opt into the single persistent orchestrator conversation instead of the v2 reducer relay |
| `ARCHON_SPEC_PATH` | Explicit path to a game-theory routing spec, searched after the workspace-local location |
| `ARCHON_PLUGIN_SEED_DIR` | Colon-separated list of directories seeded into the plugin search path |
| `ARCHON_REGISTRY_URL` | Managed-agent registry URL surfaced in `/managed-agents` help and status output |
| `ARCHON_REMOTE_URL` | Remote session URL used by `/session` to render the QR code; set automatically by the `--remote-url` flag |
| `ARCHON_WEB_DEV` | Set to `1` to run the web workbench API in development mode |

## Test and diagnostic fixtures

Set these only for local diagnostics and offline tests; they bypass live provider calls.

| Variable | Description |
|---|---|
| `ARCHON_STOOQ_CSV_URL` | Override the Stooq CSV endpoint used by trading data ingest |
| `ARCHON_TRADINGVIEW_OHLCV_FIXTURE` | Read TradingView OHLCV responses from a local fixture file instead of the live MCP call |
| `ARCHON_TRADINGVIEW_SNAPSHOT_FIXTURE` | Read TradingView snapshot payloads from a local fixture file instead of the live MCP call |
| `ARCHON_LLM_REPLAY` | `record` or `replay` — see below |
| `ARCHON_LLM_CASSETTES` | Cassette directory, default `.archon/cassettes` |

### Recording and replaying LLM exchanges

Record once against a live provider, then run the same work offline:

```bash
ARCHON_LLM_REPLAY=record ARCHON_LLM_CASSETTES=./cassettes archon -p "your prompt"
```

```bash
ARCHON_LLM_REPLAY=replay ARCHON_LLM_CASSETTES=./cassettes archon -p "your prompt"
```

One JSON file per request, named by a hash of the request. The file holds the
model, every stream event with its original chunk boundaries, and the exact
canonical form that was hashed — so when a replay misses, the two requests can
be diffed rather than guessed at.

The hash ignores what varies between runs without changing the question: tool
call ids, cache breakpoints, the `archon_spill` locator, the encrypted
reasoning blob, the tracing origin marker, and the `run_id`/`session_id` in the
`archon_runtime` envelope. It keeps everything that changes the answer —
the model, the system prompt, the messages, the tool schemas, thinking and
effort settings, and the turn counters.

Replay mode never reaches the network. A missing cassette is an error naming
the digest, the directory and the command that would record it; it is not a
fallthrough to a live call, because a test that quietly stopped exercising its
recorded path would otherwise still pass. Recording keeps the real provider's
own data-flow classification, so a recording run is gated exactly as the live
call it is making would be.

## Inherited from the environment

| Variable | Description |
|---|---|
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
2. `ARCHON_DOCS_OPENAIKEY` env (docs embeddings only)
3. `ARCHON_MEMORY_OPENAIKEY` env (memory and docs embeddings)
4. `[llm.openai] api_key` in config

Docs indexing follows `[memory].embedding_provider` unless
`ARCHON_DOCS_EMBEDDING_PROVIDER` is set. If no key is available, `auto` uses
local fastembed. `docs index` counts candidates before loading local fastembed,
so an empty index pass exits quickly.

For large local document indexing on a Mac, export the indexing profile before
starting `archon` or the TUI so child commands inherit it:

```bash
export ARCHON_DOCS_EMBEDDING_PROVIDER=local
export ARCHON_DOCS_EMBEDDING_LOAD_TIMEOUT_SECS=1800
export ARCHON_DOCS_INDEX_EMBEDDING_WORKERS=2
export ARCHON_DOCS_FASTEMBED_INSTANCES=2
export ARCHON_DOCS_INDEX_MAX_IN_FLIGHT_BATCHES=2
export ARCHON_DOCS_INDEX_WRITER_BATCH_SIZE=256
```

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
