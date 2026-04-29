# Environment variables

| Variable | Description |
|---|---|
| `ANTHROPIC_API_KEY` | Claude API key (unless using OAuth) |
| `ANTHROPIC_BASE_URL` | Override API endpoint (LiteLLM, Ollama, etc.) |
| `ARCHON_API_KEY` | Alias for `ANTHROPIC_API_KEY` |
| `ARCHON_OAUTH_TOKEN` | Pre-set OAuth bearer token (skips login) |
| `ANTHROPIC_AUTH_TOKEN` | Legacy bearer token alias |
| `OPENAI_API_KEY` | OpenAI API key for embeddings, LLM provider, and STT |
| `ARCHON_MEMORY_OPENAIKEY` | Alias for `OPENAI_API_KEY` (memory embeddings only) |
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
- [Authentication setup](../getting-started/installation.md)
