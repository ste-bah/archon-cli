# Data locations

Where archon-cli writes everything.

## Per-user state

| Linux/macOS | Windows | Purpose |
|---|---|---|
| `~/.config/archon/` | `%APPDATA%\archon\` | Configuration |
| `~/.config/archon/config.toml` | `%APPDATA%\archon\config.toml` | User config |
| `~/.config/archon/oauth.json` | `%APPDATA%\archon\oauth.json` | OAuth tokens |
| `~/.config/archon/.mcp.json` | `%APPDATA%\archon\.mcp.json` | Global MCP server config |
| `~/.local/share/archon/` | `%APPDATA%\archon\share\` | State and data |
| `~/.local/share/archon/sessions.db` | `...\sessions.db` | Session metadata + journal (CozoDB) |
| `~/.local/share/archon/sessions/<id>/` | `...\sessions\<id>\` | Per-session transcript + artefacts |
| `~/.local/share/archon/logs/<id>.log` | `...\logs\<id>.log` | Per-session log file |
| `~/.local/share/archon/checkpoints.db` | `...\checkpoints.db` | File snapshots (CozoDB) |
| `~/.local/share/archon/memory.db` | `...\memory.db` | Memory graph (CozoDB) |
| `~/.local/share/archon/identity-cache.json` | `...\identity-cache.json` | Beta header probe cache |
| `~/.local/share/archon/cron.db` | `...\cron.db` | Scheduled tasks (CozoDB) |
| `~/.local/share/archon/plugins/` | `...\plugins\` | System-installed plugins |
| `~/.local/share/archon/skills/` | `...\skills\` | User-installed skills |

XDG environment variable overrides (Linux/macOS):
- `XDG_CONFIG_HOME` overrides `~/.config` base
- `XDG_DATA_HOME` overrides `~/.local/share` base

archon-cli specific overrides:
- `ARCHON_DATA_DIR` overrides per-user state base
- `ARCHON_SESSIONS_DIR` overrides session directory
- `ARCHON_CONFIG` overrides config file path

## Per-project state

`<workdir>/.archon/`:

| Path | Purpose |
|---|---|
| `config.toml` | Project-level config (overrides user config) |
| `config.local.toml` | Local-only overrides (gitignored by convention) |
| `agents/` | Project-local custom agents (`.md` flat-file YAML or TOML manifests) |
| `teams.toml` | Multi-agent team definitions |
| `skills/<name>.toml` | User-authored skills |
| `plugins/` | Project-local plugins |
| `pipelines/<session-id>/` | Pipeline session state (specifications, artefacts, ledger) |
| `settings.json` | Hooks definitions (alternative to TOML) |
| `lsp.toml` | LSP server overrides |

Also (backward compat):
- `<workdir>/.claude/settings.json` — read for hooks, deprecated

## Project root markers

`<workdir>/`:

| File | Purpose |
|---|---|
| `ARCHON.md` | Project-level instructions to archon (loaded into system prompt) |
| `CLAUDE.md` | Backward compat alternative to ARCHON.md |
| `.mcp.json` | Project-local MCP server config (overrides global) |

## CozoDB databases

archon-cli uses CozoDB (file-based variant) for structured persistence. Each `.db` file is a self-contained SQLite-like database. Inspect with:

```bash
# CozoDB CLI (cargo install cozo-bin)
cozo restore ~/.local/share/archon/sessions.db
> :all
```

Or read schemas in `crates/archon-pipeline/src/learning/schema.rs` (learning) and per-crate equivalents.

## Cleanup

To reset everything (lose all sessions, memory, checkpoints):

```bash
rm -rf ~/.local/share/archon
rm -rf ~/.config/archon
```

To reset just sessions but keep memory:
```bash
rm ~/.local/share/archon/sessions.db
```

## Migrating between machines

Copy these to migrate full state:
- `~/.config/archon/` (configs + OAuth)
- `~/.local/share/archon/` (state + memory + sessions)

Both are POSIX-compatible (with file locks released). Tar and copy:
```bash
tar czf archon-state.tar.gz ~/.config/archon ~/.local/share/archon
```

OAuth tokens are tied to the device that originated them but may continue functioning on other machines. If they don't, run `archon login` on the target.

## See also

- [Configuration](../reference/config.md) — config file precedence
- [Session management](session-management.md) — session storage details
- [Troubleshooting](troubleshooting.md) — common path-related issues
