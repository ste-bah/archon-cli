# Cognitive Commands

The `archon cognitive` namespace inspects and runs the Cognitive Executive Loop.
Inside the TUI, use the same forms as `/cognitive ...`.

| Command | TUI form | Purpose |
|---|---|---|
| `archon cognitive status` | `/cognitive status` | Print safe counters, latest tick, proposals, and recent decision/reflection summaries |
| `archon cognitive status --json` | `/cognitive status --json` | Emit the same status as JSON |
| `archon cognitive tick` | `/cognitive tick` | Run one bounded governed maintenance pass |
| `archon cognitive tick --json` | `/cognitive tick --json` | Emit the tick report as JSON |
| `archon cognitive inspect <decision-id>` | `/cognitive inspect <decision-id>` | Inspect a single executive decision |
| `archon cognitive inspect --session <id> --limit 20` | `/cognitive inspect --session <id> --limit 20` | List recent decisions for a session |
| `archon cognitive self-model` | `/cognitive self-model` | Show domain trust and caution rules |
| `archon cognitive self-model --domain coding --domain ci` | `/cognitive self-model --domain coding --domain ci` | Scope self-model output to specific domains |
| `archon cognitive reflections --limit 20` | `/cognitive reflections --limit 20` | List safe reflection summaries |

`/cognitive` or `/cognitive open` opens the TUI Executive State pane. The pane is
read-only and shows compact state only: counts, selected candidate ids, policy
summaries, verification summaries, proposal counts, and safe lessons.

## Privacy

These commands do not print raw chain-of-thought. Decision inspection shows the
selected candidate, rejected candidate count, policy verdict, verification
contract, and user-visible summary. Reflection inspection shows compact lessons
and whether a governed proposal may be generated.

## Typical checks

```bash
archon cognitive status
archon cognitive inspect --session <session-id> --limit 10
archon cognitive reflections --limit 20
archon cognitive tick
```

Use `--json` when wiring the status into scripts or the web workbench.
