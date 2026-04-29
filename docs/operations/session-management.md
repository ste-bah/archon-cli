# Session management

Sessions store full message history, git branch, working directory, token usage, cost, and a name in CozoDB at `~/.local/share/archon/sessions.db`.

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

## Recovery from crash

If archon crashes mid-turn, the session journal is intact (CozoDB transactions). On restart:

```bash
archon -c                          # auto-resume picks up where you left off
```

Sessions interrupted during tool calls reach a "tool error / retry" state; the agent receives a tool failure result and can decide to retry or proceed.

## See also

- [Remote control](remote-control.md) — share sessions via WebSocket / web UI
- [CLI flags](../reference/cli-flags.md) — full session flag list
