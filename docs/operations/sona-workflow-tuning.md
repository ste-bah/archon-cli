# SONA workflow tuning

Four `[workflow.generated]` limits can be learned per task class from recorded
run outcomes instead of being read verbatim from your config. This page covers
what that changes, when it applies, and how to find out why a particular run got
a particular value.

For the config keys themselves see
[`[workflow.generated]`](../reference/config.md#workflowgenerated-generated-dynamic-workflows).

## It is off unless you turn it on

Tuning requires **both** toggles:

```toml
[learning.sona]
enabled = true              # default true
pipeline_recording = true   # default false
```

`pipeline_recording` defaults to `false`
(`crates/archon-core/src/config/learning.rs:38`), so tuning is off out of the
box and every run gets your configured values exactly as written.

Reading a learned weight is gated on the same consent as writing one
(`src/command/workflow_live_sona_tuning.rs:70`). A project that never consented
to recording has no evidence and would get baselines anyway; failing the gate up
front makes that explicit rather than opening a store to discover it.

## What gets tuned

Exactly four fields, and no others
(`crates/archon-core/src/config/generated_tuning.rs:41`):

| Parameter | Config default | Learner floor | Learner ceiling |
|---|---|---|---|
| `max_repair_iterations` | `3` | `2` | `6` |
| `max_investigation_iterations` | `3` | `2` | `6` |
| `verification_branch_timeout_secs` | `14400` | `7200` | `28800` |
| `host_call_timeout_secs` | `7200` | `1800` | `14400` |

The learner's bounds are deliberately narrower than the config validation range,
so a clamp is a backstop rather than the normal operating point. A tuner sitting
permanently on a clamp is indistinguishable from a tuner with a sign error, and
the two are meant to look different.

Learning is keyed per **task class** — `refactor`, `bug-hunt`, `migration`,
`review`, `greenfield` (`crates/archon-topology/src/task_hash.rs:72`) — so
evidence from refactors does not move the budgets a greenfield run gets.

## The five-observation gate

**Below five recorded outcomes on a `(task class, parameter)` key, there is no
weight at all and your configured value stands verbatim.**

`MIN_OBSERVATIONS = 5`
(`crates/archon-pipeline/src/learning/sona/tuning.rs:57`). The reasoning: below
five, one anomalous run is at least a fifth of the entire signal, and a budget
moved by one bad afternoon is worse than a static default because it looks
principled.

Under the threshold the tuner reports no weight — `None`, not a weight of zero.
The distinction is load-bearing. `decide` returns early with
`TuningSource::InsufficientEvidence` and the applied value is still the baseline
it was initialised to (`generated_tuning.rs:269`). **There is no exploration
term.** An unproven weight is not a weight, so the value is not nudged, sampled,
or averaged toward anything.

The same fail-closed path is taken when the learner rolled back on drift
(`TuningSource::DriftRolledBack`) or produced a non-finite weight — a learner bug
resolves to your configured value.

### Above the gate

There is no operator/learned blend. The applied value is a bounded multiplicative
deflection of your baseline (`generated_tuning.rs:281`):

```rust
let proposed = f64::from(baseline) * (1.0 + weight.clamp(-1.0, 1.0) * TUNING_SPAN);
```

`TUNING_SPAN` is `0.5` (`generated_tuning.rs:38`), so even a fully saturated
weight moves a value at most ±50% of what you configured, before the floor and
ceiling clamp it.

The observation count is **reported but never used to scale the value**
(`generated_tuning.rs:89`). A bigger `n` makes a weight trustworthy, not bigger.

## The timeouts only ratchet upward

The two timeout budgets are recorded as a ratchet: a run that timed out records
upward pressure, and a run that did not records *neutral* pressure rather than
downward pressure (`src/command/workflow_live_sona_tuning_outcome.rs:224`):

```rust
/// A timeout budget only ever ratchets up. See the module docs.
fn ratchet_pressure(timed_out: bool) -> f64 {
    if timed_out { 1.0 } else { NEUTRAL_PRESSURE }
}
```

`NEUTRAL_PRESSURE` is `0.5`, which is exactly the value the gradient calculation
subtracts, so a clean run contributes an observation that counts toward the
five-observation gate but moves the weight by precisely zero. **No number of
successful runs can shorten a timeout.** The invariant is pinned by a test that
feeds 200 neutral observations and asserts the timeout is still `14400`
(`src/command/workflow_live_sona_tuning_tests.rs:152`).

The rationale is that a verifier which runs out of clock does not fail honestly.
It disappears, and its silence has been observed voiding an already-accepted
remediation.

**The two iteration budgets are not ratchets.** They move in both directions
within `2..=6`: a run that resolved without ever entering the loop is genuine
evidence that the budget can come down
(`workflow_live_sona_tuning_outcome.rs:215`).

One further upward-only move is not the learner at all:
`enforce_verification_invariant` (`generated_tuning.rs:315`) raises
`verification_branch_timeout_secs` to match `host_call_timeout_secs` if learning
left it lower, and records that as `TuningSource::RaisedByVerificationInvariant`.

## "Why did this run get that value?"

There is **no** `archon sona` command and no explain or inspect verb for tuning.
The `archon learning` namespace has only `gnn status` and `tick`
(`src/cli_args/data_actions_agent.rs:32`), and `/learning-status` reports only
whether SONA is enabled — not weights, observation counts, or decisions.

You do not need to read the learning store by hand. Three surfaces answer the
question:

### 1. The run's own output, before work starts

A live run prints its tuned limits as its first output
(`src/command/workflow_live.rs:249`):

```
SONA-tuned generated limits (task class: refactor)
- max_repair_iterations: 3 -> 4 (learned, weight +0.3412, 12 observation(s))
```

Only parameters that actually moved are printed
(`src/command/workflow_live_sona_tuning.rs:117`). Silence means nothing was
learned and every value is yours. Source labels spell out the reason —
`insufficient evidence`, `learned, held at floor`, `drift detected, rolled back
to default`, and so on (`workflow_live_sona_tuning.rs:134`).

### 2. `archon workflow plan`

The planner renders the same information before you commit to a run
(`src/command/workflow_live_planner.rs:207`):

```
SONA-tuned max_repair_iterations: 3 -> 4 (weight +0.3412, 12 observation(s))
```

### 3. The persisted decision record

The durable answer, written beside the config it explains, is
`tuning_decisions` in:

```
.archon/workflows/<run-id>/v2/generated-metadata.json
```

Each entry carries `parameter`, `baseline`, `applied`, `weight`, `observations`
and `source` (`crates/archon-core/src/config/generated_tuning.rs:99`). The field
is omitted entirely on runs where nothing moved, so its absence means the run got
your configured values.

This is what makes a run directory self-explaining months later: it holds both
what the run used and why.

## Where the evidence lives

`<project>/.archon/learning-state.db`
(`src/command/workflow_live_sona_tuning.rs:58`) — a CozoDB store over a SQLite
file, **per project**, not global.

One row is recorded per observation on a route of the form
`tuning/generated/<class>/<parameter_key>`. Weights are not persisted; they are
recomputed by replaying the rows on every run
(`crates/archon-pipeline/src/learning/sona/tuning.rs:23`).

Reading never creates the file: if it does not exist, the baseline is returned
(`workflow_live_sona_tuning.rs:166`). Creating it to read zero rows would leave a
file behind implying learning had happened.

## Drift protection

Before trusting replayed history, the tuner checkpoints and rolls back if
divergence reaches `0.5` (`tuning.rs:157`), and it refuses to persist a
candidate batch that drifts by the same measure (`tuning.rs:206`). A rolled-back
key reports no weight, which means your configured value. A zero-norm prior is
exempt so a fresh key can accept its first observations.

## One hard isolation rule

**SONA weights may never reach requirement-satisfaction logic.** A learned
number decides how long a verifier may run and how many times a loop may retry;
it never decides whether a requirement is met. That separation is enforced by a
dedicated test file
(`src/command/workflow_live_sona_tuning_isolation_tests.rs`), not by convention.

## Related

- [`[workflow.generated]`](../reference/config.md#workflowgenerated-generated-dynamic-workflows)
  — the four fields and their validated ranges.
- [Learning systems](../architecture/learning-systems.md#sona-self-organizing-network-architecture)
  — SONA as a trajectory store.
- [Dynamic workflows](../architecture/dynamic-workflows.md) — the runs these
  limits bound.
