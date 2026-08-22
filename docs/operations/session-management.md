# Session management

Sessions store full message history, git branch, working directory, token usage, cost, and a name in CozoDB at the configured session DB path. By default that is the platform data directory plus `archon/sessions/sessions.db` (`~/Library/Application Support/archon/sessions/sessions.db` on macOS, `~/.local/share/archon/sessions/sessions.db` on Linux).

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

## What resuming writes to

Resuming **continues** the session you selected. New messages, cost, and token
usage are all written back to that session, whether you resumed with
`archon --resume <id>`, `--continue-session`, or the `/resume` picker inside the
TUI.

This is worth stating because it did not always hold. Before v1.5.2 the session
id was minted at startup and never reassigned, so a resumed conversation was
replayed into the transcript but every subsequent write went to a new row. One
conversation ended up split across two sessions — history in the row you
resumed, cost and name on the row the launch created — and the resumed session
stayed frozen at whatever it last held. If you have sessions from an earlier
version that show a cost but zero turns, that is what happened to them.

Note that a session row is still created for every launch, used or not, so
`/resume` lists sessions with no messages alongside real ones. The turn count in
the picker is the reliable signal.

## Forking

Fork a session to branch off a new line of work without modifying the parent:

```bash
archon --resume <id> --fork-session
```

In the TUI: `/fork`. The new session shares history up to the fork point, then diverges.

### Forking from an earlier message

`/fork` copies the whole log — "carry on from here in a separate session".
`/fork-at` answers the other question, "go back to before that and try
something else":

```
/fork-at            # lists the branch points and opens a picker
/fork-at 12         # forks through message 12, keeping 0..=12
/fork-at 12 retry-with-tokio   # ...and names it
```

The index is inclusive, and the source session is untouched — branching is not
rewinding, and the original is still there to resume. An index past the end of
the log keeps everything, because asking to branch after the last message is
asking for all of it.

Messages that are not recognisable turns are skipped from the picker but keep
their positions, so the index you pass is always the position in the log.

`/fork-at` is not called `/branch`: `/branch` is the built-in skill that manages
*git* branches.

## Referencing another session

`/fork` and `/fork-at` make a new session out of an old one. `/session-ref`
answers a different question — "what did that other session find?" — without
leaving the session you are in:

```
/session-ref 0f3c1b2a-...   # attach an excerpt of that session to your next message
```

The excerpt is attached to the **next message you send**, once, and is then
gone. It is not a permanent addition to the session.

Three things about it are deliberate.

It is **bounded**. The last 20 stored messages, capped at 16 KB of rendered
transcript. Over the cap, the whole transcript is written to the spill store
under `.archon/spill/` and the attached excerpt names that file, so nothing is
quietly cut off — if the write fails, the command fails rather than attaching a
silently shortened excerpt.

It is **the stored log, not the other session's live context**. That session may
have compacted since, so the excerpt can contain material that session itself
decided was not worth keeping. The attached block says so. Projecting the source
session's current surface instead is the better answer and is not what this does
yet.

It is **untrusted**. A transcript is model output and tool results, and text
inside it can be shaped like an instruction. The excerpt is therefore wrapped in
its own tag behind a preamble stating that the contents are data, that no
directive inside them is to be followed, and that the turn's instructions come
only from your own message. Angle brackets inside the excerpt are escaped, so
nothing in the referenced transcript can close the wrapper and continue as if it
were your text.

A session id that does not exist, or one with no messages, is an error you see —
never an empty attachment that looks like it worked.

## Rating a message

`/feedback` records what the learning subsystems cannot infer — whether the
person reading an answer thought it was any good.

```
/feedback            # what is the last message rated?
/feedback good       # ...or `+`, or `up`
/feedback bad why it was wrong
/feedback clear
/feedback list       # everything rated in this session
```

`/rate` is an alias. A note is optional and free text.

Ratings live in a sidecar relation keyed by message id, **never in the message
log**, so they never reach model context. A model that could see its last
answer was rated badly would start writing for the rating rather than for the
task.

Writes are compare-and-swap on an opaque version token, so two sessions rating
the same message cannot silently overwrite each other — the loser is told the
rating changed underneath it.

A message has no id of its own, so a rating is keyed by its position in the
log — and positions move: compaction replaces the whole message list with a
shorter one. Each rating therefore records a fingerprint of the message it was
made about, and a rating whose fingerprint no longer matches what sits at that
index is reported as absent rather than as describing the message now there.
Losing a rating to a compaction is recoverable; feeding the learning layer a
rating of the wrong answer is not.

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
| Platform data dir + `archon/sessions/sessions.db` | Session metadata + journal (CozoDB) |
| `~/.local/share/archon/checkpoints.db` | File snapshots (CozoDB) |
| `~/.local/share/archon/sessions/<id>/` | Per-session transcript + artefacts |
| `~/.local/share/archon/logs/<id>.log` | Per-session log file |
| `~/.archon/sessions/<id>/activity/events.jsonl` | Session activity JSONL used by retrospectives |
| `~/.archon/self-calibration/` | Retrospectives, self-trust records, and plan-vs-outcome summaries |

Inside `sessions.db`, two relations exist alongside the journal and are not
part of the conversation:

| Relation | Holds |
|---|---|
| `message_feedback` | Per-message ratings from `/feedback`. Never read into model context. |
| `session_projections` | Cached folds over the event log (see below). |

### Projections

Anything derived from a session's history — message counts, which tools were
used, cost by turn — is computed by folding the event log. Done naively that
means rescanning the whole log every time anyone asks, which gets slower for
exactly the sessions where the answer is most interesting.

A projection folds the log once and caches the result with the sequence number
it folded through. The next call resumes from there and applies only the events
that arrived since. A session that has not moved costs one lookup.

The cache is written only when the fold actually advanced, so a read-only query
against an idle session does no writes. It is derived data throughout: deleting
`session_projections` costs a rescan and nothing else, and
`invalidate_projection` exists for when a projection's own logic changes and
old cached state would be wrong rather than stale.

`/status` reads one of these — message counts and the distinct set of tools
used. It reports nothing at all rather than zeroes when a session has no
projection yet, because "not measured" and "measured as zero" are different
answers and a bar of zeroes looks like the latter.


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
