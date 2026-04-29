# First run

What to expect when you launch `archon` for the first time.

## What happens on first launch

1. archon-cli generates a config file at `~/.config/archon/config.toml` with commented defaults.
2. It looks for credentials in this order:
   - OAuth token at `~/.config/archon/oauth.json` (from a prior `archon login`)
   - `ARCHON_OAUTH_TOKEN` / `ANTHROPIC_AUTH_TOKEN` env vars
   - `ANTHROPIC_API_KEY` / `ARCHON_API_KEY` env vars
3. If no credentials are found, archon prompts you to run `archon login` or set an API key.
4. The TUI opens with built-in defaults (INTJ personality, `dracula` theme, default permission mode).

## Where data lives

| Path | Purpose |
|---|---|
| `~/.config/archon/config.toml` | User config (model, identity, personality, permissions) |
| `~/.config/archon/oauth.json` | OAuth tokens (when using `archon login`) |
| `~/.local/share/archon/` (Linux/macOS) | Per-user state: sessions, memory graph, logs |
| `%APPDATA%\archon\` (Windows) | Per-user state on Windows |
| `~/.local/share/archon/sessions/` | Session checkpoints + transcripts |
| `~/.local/share/archon/logs/` | Per-session log files (default level: `info`) |
| `<workdir>/.archon/` | Project-local config, agents, hooks, plugins |
| `<workdir>/.archon/config.toml` | Project-level config overrides |
| `<workdir>/.archon/agents/` | Project-local custom agents |
| `<workdir>/.archon/teams.toml` | Multi-agent team definitions |
| `<workdir>/.archon/skills/` | User-authored skills |

Project-local config layers on top of user config — see [Configuration](../reference/config.md) for precedence.

## First commands to try

In the TUI:

```
/help                  # list all 65 primary slash commands
/status                # session info: model, effort, tokens used
/cost                  # estimated session cost
/themes                # cycle theme
/agent list            # list all discovered agents (built-in + flat-file + plugins)
/learning-status       # show which learning subsystems are enabled
```

## Common gotchas

| Symptom | Cause | Fix |
|---|---|---|
| TUI shows `(no auth)` and refuses to send messages | No credentials found | Run `archon login` or `export ANTHROPIC_API_KEY=...` |
| `429 Too Many Requests` on every send | Rate limit on your account / shared IP | Wait, or check `/status` for retry timing |
| `/run-agent foo` returns `Blocked. The Agent tool requires elevated permissions` | Permission mode does not allow `Agent` tool | Switch mode: type `/permissions auto` or pass `--permission-mode auto` at launch |
| Pipeline agent dispatch panics with `blocking_lock`-style error | Pre-v0.1.13 bug — should not occur on current builds | Upgrade to latest release |
| Memory recall returns empty | First run; no memories yet | Memories accrue from agent activity. Use `/store-memory <text>` to seed |
| Theme looks wrong colours in terminal | Terminal does not support truecolor | `archon --list-themes` to see compatible themes; pick a 16-color theme |
| Slow startup (~5s on first launch) | First-time CozoDB schema initialization | Subsequent launches are <500ms |

## Logs

Per-session log files live under `~/.local/share/archon/logs/<session-id>.log`. Default level is `info`; bump with:

```bash
archon --debug api,llm,memory       # specific categories
archon --verbose                    # info → debug
RUST_LOG=archon=trace archon        # full tracing
```

The log file is human-readable and includes timestamps, request/response correlation IDs, and tool call summaries. Errors are written to both stdout (in print mode) and the log file.

## Sanity loop

A 10-second smoke test:

```bash
# 1. Confirm version
archon --version

# 2. List built-in tools (should be 43)
archon --list-tools | wc -l

# 3. Print mode against a simple file
echo "fn main() { println!(\"hi\"); }" > /tmp/hi.rs
archon -p "what does this do?" --append-system-prompt-file /tmp/hi.rs --output-format text

# 4. Check session was written
ls -la ~/.local/share/archon/sessions/ | head -3
```

If any step errors out, see [Troubleshooting](../operations/troubleshooting.md).

## Next steps

- [Slash commands reference](../reference/slash-commands.md) — full 65-command catalogue
- [Configuration](../reference/config.md) — every config section explained
- [Cookbook](../cookbook/) — task-oriented walkthroughs (god-code pipeline, memory-driven coding, etc.)
