# Environment variables

| Variable | Description |
|---|---|
| `ANTHROPIC_API_KEY` | Claude API key (unless using OAuth) |
| `ANTHROPIC_BASE_URL` | Override API endpoint (LiteLLM, Ollama, etc.) |
| `ARCHON_API_KEY` | Alias for `ANTHROPIC_API_KEY` |
| `ARCHON_OAUTH_TOKEN` | Pre-set OAuth bearer token (skips login) |
| `ANTHROPIC_AUTH_TOKEN` | Legacy bearer token alias |
| `OPENAI_API_KEY` | OpenAI API key for embeddings, LLM provider, and STT |
| `GOOGLE_API_KEY` | Google Generative Language API key for Gemini VLM image descriptions |
| `ARCHON_MEMORY_OPENAIKEY` | Alias for `OPENAI_API_KEY` (memory embeddings only) |
| `ARCHON_CODEX_DISABLED` | Disable Codex provider resolution when set to `1`, `true`, or `yes` |
| `ARCHON_CODEX_BASE_URL` | Override Codex backend URL for local mocks or diagnostics |
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
| `ARCHON_DATA_DIR` | Override per-user state dir (default: `~/.local/share/archon`) |
| `ARCHON_SESSIONS_DIR` | Override session directory |
| `ARCHON_NO_TUI` | Force headless mode |
| `EDITOR` | Used by `/commit` and skill workflows that open an editor |
| `SHELL` | Inherited by `Bash` tool subprocesses |
| `HOME` | Used to resolve `~/.config/archon/` and `~/.local/share/archon/` |
| `XDG_CONFIG_HOME` | Linux/macOS: overrides `~/.config` base |
| `XDG_DATA_HOME` | Linux/macOS: overrides `~/.local/share` base |
| `APPDATA` | Windows: per-user state base |
| `SSH_AUTH_SOCK` | Used by `archon remote ssh` for agent forwarding |

## Resolution order for credentials

1. `~/.config/archon/oauth.json` (from `archon login`)
2. `ARCHON_OAUTH_TOKEN` env
3. `ANTHROPIC_AUTH_TOKEN` env (legacy)
4. `ANTHROPIC_API_KEY` env
5. `ARCHON_API_KEY` env (alias)

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
