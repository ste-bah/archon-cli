# Session management

Sessions store full message history, git branch, working directory, token usage, cost, and a name in CozoDB at `~/.local/share/archon/sessions.db`.

> **TUI parity.** The session-management commands shown below as `archon --resume`, `archon --continue-session`, etc. are all available inside the TUI as slash commands: `/resume`, `/sessions`, `/rename`, `/fork`, `/tag`. See [CLI and TUI Command Parity](../cookbook/real-world-evidence-engine.md#cli-and-tui-command-parity). The CLI flags exist primarily for fresh-launch invocations and scripting; in-session work prefers the slash forms.

## Auto-resume

By default archon-cli auto-resumes the most recent session in the current working directory:

```toml
[session]
auto_resume = true
```

Disable per-invocation:
```bash
archon --no-resume
```

## Resume by ID, name, or prefix

```bash
# Full UUID
archon --resume 8383f1ea-1234-5678-abcd-000000000000

# UUID prefix
archon --resume 8383f1ea

# Session name
archon --resume my-feature-work

# List all and pick interactively
archon --resume
```

## Continue most recent

```bash
archon --continue-session                # or -c
archon -c                                # shorthand
```

## Forking

Fork a session to branch off a new line of work without modifying the parent:

```bash
archon --resume <id> --fork-session
```

In the TUI: `/fork`. The new session shares history up to the fork point, then diverges.

## Naming sessions

```bash
archon --session-name "oauth-refactor"
```

In the TUI: `/rename oauth-refactor`.

Names can be searched via `archon --sessions --search oauth`.

## Tags

```
/tag urgent
/tag review-needed
```

Tags are searchable. Toggle by repeating the command.

## Listing and searching

```bash
# CLI session search
archon --sessions                                          # list all
archon --sessions --search "oauth"                         # text search
archon --sessions --branch main --after 2026-04-01         # filter
archon --sessions --stats                                  # aggregate stats
archon --sessions --delete <id>                            # remove
```

In the TUI: `/sessions` (skill).

## Checkpointing & file snapshots

archon-cli snapshots every file the agent modifies, keyed by turn number. Storage: `~/.local/share/archon/checkpoints.db` (CozoDB).

| Command | Purpose |
|---|---|
| `/checkpoint` | Save a named checkpoint |
| `/rewind` | Jump back to a previous checkpoint (interactive picker) |
| `/restore` | List all modified files with checkpoints |
| `/restore <FILE>` | Show diff and restore to latest snapshot |
| `/restore <FILE> <TURN>` | Restore to a specific turn number |
| `/restore --all` | Restore all modified files |
| `/undo` | Undo last file modification |

The `checkpoint_diff` module computes line-level diffs between versions for inspection before restore.

## Session storage details

| Path | Purpose |
|---|---|
| `~/.local/share/archon/sessions.db` | Session metadata + journal (CozoDB) |
| `~/.local/share/archon/checkpoints.db` | File snapshots (CozoDB) |
| `~/.local/share/archon/sessions/<id>/` | Per-session transcript + artefacts |
| `~/.local/share/archon/logs/<id>.log` | Per-session log file |
| `~/.archon/sessions/<id>/activity/events.jsonl` | Session activity JSONL used by retrospectives |
| `~/.archon/self-calibration/` | Retrospectives, self-trust records, and plan-vs-outcome summaries |

## Recovery from crash

If archon crashes mid-turn, the session journal is intact (CozoDB transactions). On restart:

```bash
archon -c                          # auto-resume picks up where you left off
```

Sessions interrupted during tool calls reach a "tool error / retry" state; the agent receives a tool failure result and can decide to retry or proceed.

## Activity retrospectives

Archon also writes per-session activity JSONL for agent/tool events. v1.0.0 can
read those logs back, and v1.0.1 adds provider-neutral analyzer modes:

```bash
archon self retrospective <session-id>
archon self retrospective <session-id> --analyzer heuristic
archon self retrospective <session-id> --analyzer llm
archon self trust status
archon self plans inspect <session-id>
```

The retrospective command reads `~/.archon/sessions/<id>/activity/events.jsonl`,
writes artifacts under `~/.archon/self-calibration/`, and attempts to promote
high-signal lessons into memory and governed LearningEvents. The default hybrid
extractor combines deterministic local rules with an LLM-assisted pass that uses
the active configured provider. If the provider is unavailable, Archon records
the analyzer note and falls back to deterministic candidates. LLM candidates are
validated against real event ids and filtered for confidence, duplicates, and
secret-shaped content before they can update memory or self-trust.

## See also

- [Web workbench](web-workbench.md) — inspect sessions, learning, pipelines, and evidence in the browser
- [Remote control](remote-control.md) — share sessions via WebSocket / web UI
- [CLI flags](../reference/cli-flags.md) — full session flag list
