# Context compaction

archon-cli compresses older turns when context fills up so long sessions stay productive. Compaction is automatic by default and tunable.

## Automatic compaction

When context fill ratio exceeds the threshold, archon-cli runs the compaction agent in the background:

```toml
[context]
compact_threshold = 0.8             # Trigger when 80% full
preserve_recent_turns = 3           # Always keep the last N turns verbatim
prompt_cache = true                 # Anthropic prompt cache on static blocks
```

The compaction agent reads the older turns, produces a condensed summary preserving:
- Decisions and their reasoning
- File modifications and their context
- Tool call outputs that informed decisions
- Active rules / corrections

It then replaces the original turns with the summary in the session journal. The recent N turns are preserved verbatim so the model has fresh context.

## Manual compaction

```
/compact                            # Trigger immediately
```

In CLI:
```bash
archon -p "long task" --max-turns 50  # Force compaction at 50 turns
```

## Pre-compaction and post-compaction hooks

Hook events fire around compaction:

```toml
[[hooks.pre_compact]]
command = "scripts/log-pre-compact.sh"

[[hooks.post_compact]]
command = "scripts/save-summary.sh"
```

See [Hooks](../integrations/hooks.md) for the full event list.

## What gets preserved

Compaction is lossy by design — older detail is summarized, not removed. Specifically retained:

1. **System prompt** (never compacted)
2. **Active rules** (never compacted)
3. **Memory injection block** (refreshed on every turn)
4. **Recent N turns** (verbatim, configured via `preserve_recent_turns`)
5. **Compaction summary** (replaces older turns)

Compaction is idempotent — re-compacting an already-compacted session re-summarizes the existing summary alongside any new turns.

## Disabling compaction

Useful for debugging or for very short sessions:

```toml
[context]
compact_threshold = 1.0             # Effectively disabled
```

Or per-invocation:
```bash
archon --no-compact                 # if implemented; check --help
```

## Prompt cache

When `prompt_cache = true`, archon-cli sets the Anthropic prompt-cache-control flag on static blocks (system prompt, tool catalog, memory briefing). Anthropic caches these blocks server-side for ~5 minutes and bills cache-hit tokens at a fraction of input cost.

Cache hits are visible in `/usage` and `/extra-usage` reports. A high cache-hit ratio over a long session significantly reduces cost.

## Inspecting context state

```
/context                            # Current context window usage
```

Shows:
- Total tokens used vs model limit
- Breakdown by component (system, tools, memory, history)
- Last compaction timestamp + size delta
- Cache hit ratio

## See also

- [Cost, effort, fast mode](cost-effort.md) — tuning per-turn token use
- [Configuration](../reference/config.md) — `[context]` section
- [Hooks](../integrations/hooks.md) — `pre_compact` / `post_compact` events
