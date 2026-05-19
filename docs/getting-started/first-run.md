# First run

What to expect when you launch `archon` for the first time.

## What happens on first launch

1. archon-cli generates a config file at `~/.config/archon/config.toml` with commented defaults.
2. It looks for active-provider credentials. For Anthropic, the order is:
   - `~/.archon/.credentials.json` (from `archon auth login --provider anthropic`)
   - deprecated fallback `~/.claude/.credentials.json`, if the Archon file is absent
   - `ARCHON_OAUTH_TOKEN` / `ANTHROPIC_AUTH_TOKEN` env vars
   - `ANTHROPIC_API_KEY` / `ARCHON_API_KEY` env vars
3. If no credentials are found, archon prompts you to run `archon auth login --provider anthropic` or set an API key. Codex-backed sessions use `archon auth login --provider openai-codex` plus `[llm].provider = "openai-codex"`.
4. The TUI opens with built-in defaults (INTJ personality, `dracula` theme, default permission mode).

## Where data lives

| Path | Purpose |
|---|---|
| `~/.config/archon/config.toml` | User config (model, identity, personality, permissions) |
| `~/.archon/.credentials.json` | Provider credentials from `archon auth login` (Anthropic OAuth/API key, Codex OAuth, Gemini API key) |
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
/help                  # list all 78 primary slash commands
/setup-archon-skills   # interactive 8-prompt config wizard
/status                # session info: model, effort, tokens used
/cost                  # estimated session cost
/themes                # cycle theme
/agent list            # list all discovered agents (built-in + flat-file + plugins)
/learning-status       # show which learning subsystems are enabled
```

From the same project root, you can also launch the browser workbench:

```bash
archon web --port 8421 --bind-address 127.0.0.1
```

For a blank project, run `scripts/archon-init.sh` first so the web workbench
has `.archon/`, docs inboxes, policy defaults, `prds/`, and `tasks/` to inspect.
See [Web workbench](../operations/web-workbench.md) for the tab-by-tab guide.

## Common gotchas

| Symptom | Cause | Fix |
|---|---|---|
| TUI shows `(no auth)` and refuses to send messages | No credentials found for the active provider | Run `archon auth login --provider anthropic`, set `ANTHROPIC_API_KEY=...`, or configure Codex auth + `[llm].provider = "openai-codex"` |
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

# 2. List registered tools (should be 65)
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
- [Web workbench](../operations/web-workbench.md) — browser UI setup, tabs, and safety model
- [Cookbook](../cookbook/) — task-oriented walkthroughs (god-code pipeline, memory-driven coding, etc.)
