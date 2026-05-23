# Context compaction

archon-cli compresses older turns when context fills up so long sessions stay productive. Compaction is automatic by default and tunable.

## Automatic compaction

When context fill ratio approaches the threshold, archon-cli asks the active provider for a structured text-only summary of older turns, then replaces those older turns with the summary before the next request:

```toml
[context]
compact_threshold = 0.8             # Trigger when 80% full
preflight_safety_margin = 0.05      # Start before the hard threshold
output_reserve_tokens = 8192        # Leave space for the next answer
preserve_recent_turns = 3           # Always keep the last N turns verbatim
rate_limit_pressure_tokens = 120000 # Compact huge requests before provider retry pressure
rate_limit_pressure_body_bytes = 320000
large_request_retry_body_bytes = 320000
prompt_cache = true                 # Anthropic prompt cache on static blocks
prompt_cache_mode = "explicit"
prompt_cache_ttl = "5m"
```

The provider-backed summary preserves:
- Decisions and their reasoning
- File modifications and their context
- Tool call outputs that informed decisions
- Active rules / corrections

Archon then replaces the original turns with the summary in the session journal. The recent N turns are preserved verbatim so the model has fresh context. If the summary request itself exceeds the provider context window, Archon retries with older API-round groups removed before failing structurally.

Large tool outputs are also bounded before they are replayed to the model. The full output can still be shown in the UI/log path, but the LLM-visible transcript keeps only a bounded head/tail summary so a single verbose command cannot turn the next small prompt into a huge provider request.

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

When `prompt_cache = true`, archon-cli applies provider-aware prompt-cache hints to stable Anthropic-shape system blocks. Unsupported providers receive no `cache_control` keys. `prompt_cache_ttl = "1h"` opts into longer-lived cache entries where the provider supports them.

Cache creation/read tokens are visible in `/usage`, `/extra-usage`, `/context`, and the TUI status bar once cache tokens are observed.

## Inspecting context state

```
/context                            # Current context window usage
```

Shows:
- Total tokens used vs model limit
- The context-window source (`config-override`, `user-catalog`, `bundled-catalog`, `provider`, or `fallback`)
- Breakdown by component (system, tools, memory, history)
- Last compaction timestamp + size delta
- Cache creation/read token counts

## See also

- [Cost, effort, fast mode](cost-effort.md) — tuning per-turn token use
- [Configuration](../reference/config.md) — `[context]` section
- [Hooks](../integrations/hooks.md) — `pre_compact` / `post_compact` events
