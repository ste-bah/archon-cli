# Cost, effort, and fast mode

archon-cli surfaces token cost in real time and provides three knobs to tune latency and quality:

## Cost tracking

```
/cost                               # Token cost breakdown for current session
/usage                              # Token usage, cost, turn count
/extra-usage                        # 6-section detailed report
```

Costs are computed from token counts × Anthropic per-model pricing (cached locally; updated periodically).

## Cost limits

Configure soft warning and hard limit:

```toml
[cost]
warn_threshold = 30.0               # Warn when session cost exceeds $30
hard_limit = 50.0                   # Halt new turns above $50 (0.0 = no limit)
```

Per-invocation:
```bash
archon --max-budget-usd 5.00
```

When `hard_limit` is reached, archon refuses new turns and displays the cumulative cost. Restart to reset (or fork the session).

## Effort levels

Effort controls how much reasoning the model does per turn:

```bash
archon --effort high               # Default
archon --effort medium
archon --effort low
```

In session: `/effort high|medium|low`.

| Level | Behaviour |
|---|---|
| `high` | Extended thinking budget (16384 tokens by default), thorough reasoning |
| `medium` | Reduced thinking budget (4096 tokens), balanced |
| `low` | No extended thinking, fastest response |

Configure default and budget:
```toml
[api]
default_effort = "high"
thinking_budget = 16384
```

## Fast mode

Fast mode trades quality for latency: smaller model, smaller context, no extended thinking.

```bash
archon --fast
```

In session: `/fast` to toggle.

Best for:
- Quick fact lookups
- File search / navigation
- Status checks
- Simple slash command invocations

Not recommended for:
- Code generation
- Multi-step pipelines (`/archon-code`, `/archon-research`)
- Complex reasoning tasks

## Model selection

```bash
archon --model claude-haiku-4-5    # Override default
archon --model claude-sonnet-4-6
archon --model claude-opus-4-7
```

In session: `/model <name>` or `/model` to list.

The selected model overrides effort defaults — `--model claude-haiku-4-5 --effort high` does NOT enable extended thinking on Haiku (Haiku doesn't support it).

## Latency vs cost matrix

| Goal | Settings |
|---|---|
| Fastest, cheapest | `--fast --model claude-haiku-4-5` |
| Balanced | `--effort medium --model claude-sonnet-4-6` (default) |
| Maximum quality | `--effort high --model claude-opus-4-7` |
| Pipeline runs | `--effort high` (each pipeline agent picks its own model) |

## Per-session cost in the TUI

The status line (configurable via `/statusline`) can display:
- Total tokens used (input + output + cache hits)
- Estimated cost in USD
- Turn count
- Cache hit ratio

## Cost telemetry

Per-turn cost is logged to `~/.local/share/archon/logs/<session>.log`:

```
[INFO archon::cost] turn=42 input_tokens=4203 output_tokens=856 cache_hit=true cost_usd=0.0124
```

For aggregate stats, use `/stats` (skill) or `archon --sessions --stats`.

## See also

- [Configuration](../reference/config.md) — `[cost]` and `[api]` sections
- [CLI flags](../reference/cli-flags.md) — `--effort`, `--fast`, `--model`, `--max-budget-usd`
- [Context compaction](context-compaction.md) — reducing token use via compression
