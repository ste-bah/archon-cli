# Cognitive Commands

The `archon cognitive` namespace inspects and runs the Cognitive Executive Loop.
Inside the TUI, use the same forms as `/cognitive ...`.

| Command | TUI form | Purpose |
|---|---|---|
| `archon cognitive status` | `/cognitive status` | Print safe counters, latest tick, proposals, and recent decision/reflection summaries |
| `archon cognitive status --json` | `/cognitive status --json` | Emit the same status as JSON |
| `archon cognitive tick` | `/cognitive tick` | Run one bounded governed maintenance pass |
| `archon cognitive tick --json` | `/cognitive tick --json` | Emit the tick report as JSON |
| `archon cognitive daemon status` | `/cognitive daemon status` | Inspect background daemon state, lock path, heartbeat, and tick count |
| `archon cognitive daemon start` | `/cognitive daemon start` | Spawn the Rust daemon as a background process |
| `archon cognitive daemon stop` | `/cognitive daemon stop` | Request the running daemon to stop at the next safe checkpoint |
| `archon cognitive daemon run` | `/cognitive daemon run` | Run the daemon in the foreground for supervised service managers |
| `archon cognitive daemon run-once` | `/cognitive daemon run-once` | Run one daemon-gated pass without staying resident |
| `archon cognitive inspect <decision-id>` | `/cognitive inspect <decision-id>` | Inspect a single executive decision |
| `archon cognitive inspect --session <id> --limit 20` | `/cognitive inspect --session <id> --limit 20` | List recent decisions for a session |
| `archon cognitive self-model` | `/cognitive self-model` | Show domain trust and caution rules |
| `archon cognitive self-model --domain coding --domain ci` | `/cognitive self-model --domain coding --domain ci` | Scope self-model output to specific domains |
| `archon cognitive reflections --limit 20` | `/cognitive reflections --limit 20` | List safe reflection summaries |
| `archon cognitive gate` | — (shell only) | Judge the derived cognitive metrics against the declared release thresholds, per cohort. Exits non-zero if any segment fails |
| `archon cognitive gate --json` | — (shell only) | Emit the same gate report as JSON |

`/cognitive` or `/cognitive open` opens the TUI Executive State pane. The pane is
read-only and shows compact state only: counts, selected candidate ids, policy
summaries, verification summaries, proposal counts, and safe lessons.

## The release gate

`archon cognitive gate` is the consumer of the cognitive metric events. It reads
the derived metrics and judges them against `METRIC_THRESHOLD_VERSION` — bounds
that are compiled constants beside the metric definitions, so loosening one to
get a release out is a reviewable diff rather than a row nobody sees.

Three things it will not do:

- **It does not judge the aggregate.** Every cohort is judged separately and a
  single failing segment fails the gate; the pooled figure is judged too, but as
  one voice among the segments rather than the deciding one.
- **It does not round a thin cohort up.** A cohort under the sample floor is
  `InsufficientEvidence`, and a run where nothing could be judged is
  `NotEvaluated` — neither is a pass, because "we did not measure enough" is a
  different claim from "we measured and it was fine".
- **It does not compare across a definition change.** A threshold naming a metric
  no definition derives, or bounds orphaned by a changed formula, yield
  `DefinitionDrift`, which blocks. An incomparable check is not a pass.

The same gate runs inside the cognitive tick, before `propose_improvements`: a
degraded segment must not be the evidence base for the agent proposing changes to
its own behaviour. Blocked checks land in `report.errors` and are persisted to
the audit row, so the audit names the segment that stopped it rather than showing
an unexplained zero. An unreadable metric store fails closed.

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
archon cognitive daemon status
```

Use `--json` when wiring the status into scripts or the web workbench.

## Daemon safety

The daemon requires all of these gates:

- `[learning.cognitive.daemon].enabled = true`
- `[policy.cognitive].enabled = true`
- `[policy.cognitive].allow_autonomous_tick = true`
- `[policy.cognitive].allow_background_daemon = true`

It writes a lockfile and state file under `ledger_dir`, heartbeats while alive,
and records learning-job decisions in `learning-daemon-events.jsonl`. World
model trainer progress, including JEPA backend selection, row load, example
build, encode, fit, loss, collapse, and candidate-write stages, is recorded in
`~/.archon/world-model/ledgers/daemon-trainer-events.jsonl` and surfaced by
`archon cognitive daemon status`.

The daemon job list is implemented as a Rust trait so future maintenance jobs
can be added without turning the cognitive tick path into a monolith.
