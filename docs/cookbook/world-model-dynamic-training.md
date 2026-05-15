# World Model And JEPA Training Cookbook

This guide walks through a fresh Archon setup and shows how to get from "no
world-model data yet" to an active advisory model, then to an optional JEPA
representation model.

The short version:

```bash
archon world ingest --backfill
archon world status
archon world train --candidate
archon world eval <candidate-id>
archon world promote <candidate-id>
archon world train-jepa --candidate
archon world eval-jepa <jepa-candidate-id>
archon world promote-jepa <jepa-candidate-id>
```

Do not expect all of those commands to pass on day one. The world model needs
trace history. JEPA needs even more trace history because it is learning a
representation, not just fitting a small transition model.

## What You Are Building

Archon's world model is local and advisory. It watches redacted traces from real
Archon sessions, learns what usually happens after different actions, and can
later provide fail-open predictions such as "this action may need verification"
or "this path often retries".

There are two model kinds:

| Model kind | What it does | When to use it |
|---|---|---|
| `latent_transition` | Existing local transition model over generic embeddings. | Default, good first model, lower data requirement. |
| `jepa_transition` | JEPA trace representation plus transition model over JEPA latents. | Optional upgrade after there is enough trace history. |

The runtime advisor never blocks your foreground work. If the corpus is cold, a
checkpoint is missing, or JEPA is unavailable, Archon records the reason and
continues.

## How Long It Usually Takes

These are practical expectations, not guarantees:

| Step | Typical time | Notes |
|---|---:|---|
| Backfill existing sessions | Seconds to a few minutes | Depends on how many sessions, bundles, and activity logs exist. |
| First local FastEmbed use | A few seconds to several minutes | The local embedding model may need to initialise or download/cache on first use. |
| `archon world train --candidate` | Seconds to a few minutes | CPU is the reliable baseline. The configured trainer runtime budget is 5 minutes by default. |
| `archon world train-jepa --candidate` | Seconds to a few minutes | Larger corpora take longer. The configured JEPA trainer runtime budget is 5 minutes by default. |
| `eval-jepa` | Seconds to a few minutes | Uses the fixed FastEmbed baseline for promotion gating. |

If a command exits with a gate failure, that is usually not an error. It means
the model was written as a candidate, but Archon does not trust it enough to make
it active yet.

## Fresh Setup Checklist

Start with a normal installed Archon CLI and at least one project where you use
Archon for real work.

1. Check that the world model is enabled:

```bash
archon world status
```

Look for:

```text
Enabled:            true
Model kind:         latent_transition
Cold-start status:  cold_start (...)
Advisor status:     fail-open
```

On a brand-new setup, `cold_start` and `fail-open` are normal. They mean there
is not enough local trace history yet.

2. Confirm the important defaults in your config.

User config is normally:

```text
~/.config/archon/config.toml
```

Project-local config, when present, is:

```text
<project>/.archon/config.toml
<project>/.archon/config.local.toml
```

The defaults should look like this:

```toml
[learning.world_model]
enabled = true
model_kind = "latent_transition"
state_dim = 384
store_raw_text = false

[learning.world_model.cold_start]
min_rows = 1000
min_sessions = 50
min_observed_days = 7

[learning.world_model.jepa]
enabled = false
latent_dim = 384
min_training_examples = 2000
min_heldout_examples = 200
```

Leave `model_kind = "latent_transition"` until a JEPA candidate has passed eval
and promotion. Setting `model_kind = "jepa_transition"` too early is safe, but
the advisor will fail open until a promoted JEPA checkpoint exists.

## Step 1: Create Trace Data

Use Archon normally. Coding sessions, research sessions, tool calls, provider
events, corrections, retries, verification actions, and outcomes are the useful
material.

For a fresh setup, the simplest way to build a useful corpus is:

- run real Archon sessions for several days;
- let pipelines finish rather than killing them early when possible;
- record or accept outcomes when Archon asks;
- keep provider/runtime events enabled;
- avoid deleting `~/.archon/world-model/` unless you want to reset learning.

The world model stores redacted summaries and structured metadata. It is not
trying to reconstruct raw transcripts.

## Step 2: Backfill The Corpus

Backfill reads existing sessions, activity logs, pipeline bundles, transcripts,
and run artifacts into the local world-model store.

```bash
archon world ingest --backfill
```

For one known session:

```bash
archon world ingest <session-id>
```

Then check status:

```bash
archon world status
```

Read these lines first:

```text
Corpus rows:        <number>
Corpus sessions:    <number>
Observed days:      <number>
Cold-start status:  ...
Candidate models:   <number>
JEPA status:        ...
JEPA candidates:    <number>
Advisor status:     ...
```

A good first target is:

```text
Corpus rows:        1000 or more
Corpus sessions:    50 or more
Observed days:      7 or more
Cold-start status:  ready
```

You can train before this, but early candidates are more likely to fail eval or
give weak advice.

## Step 3: Train The Default Model First

Train the default local transition model before JEPA. It gives Archon a useful
advisor earlier and also proves the corpus is healthy.

```bash
archon world train --candidate
```

The output includes a candidate id:

```text
Candidate: world-model-candidate-...
```

Use that id in the eval command:

```bash
archon world eval world-model-candidate-...
```

If eval passes, promote it:

```bash
archon world promote world-model-candidate-...
```

Check status again:

```bash
archon world status
```

You want to see an active model and `Advisor status: ready`. If eval does not
pass, keep using Archon, backfill later, and train another candidate.

## Step 4: Enable JEPA Candidate Training

JEPA is opt-in for automatic training. Add this if you want the dynamic trainer
to create JEPA candidates when the machine is idle:

```toml
[learning.world_model.jepa]
enabled = true
```

Keep this unchanged for now:

```toml
[learning.world_model]
model_kind = "latent_transition"
```

Manual `train-jepa` works either way, but `jepa.enabled = true` makes status and
trainer behavior clearer while you are building candidates.

## Step 5: Know When JEPA Is Worth Training

Run:

```bash
archon world status
```

JEPA is worth trying when:

```text
Cold-start status:  ready
Corpus rows:        comfortably above 1000
Corpus sessions:    comfortably above 50
Observed days:      7 or more
```

JEPA is worth evaluating for promotion when training output later says:

```text
Examples: 2000 or more
```

The default promotion gate requires:

```toml
min_training_examples = 2000
min_heldout_examples = 200
```

If you have fewer examples, JEPA can still write a candidate, but `eval-jepa`
will keep it candidate-only.

## Step 6: Train JEPA Manually

Run:

```bash
archon world train-jepa --candidate
```

The important output lines are:

```text
Candidate: jepa-world-model-candidate-...
Model kind: jepa_transition
Rows loaded: ...
Examples: ...
Loss improved: true
Collapse gate: true (std=..., rank_ratio=...)
Horizon gate: true
Manifest: ...
Checkpoint: ...
```

How to read that:

| Output | Meaning |
|---|---|
| `Examples` | Number of trace-window training examples. For promotion, aim for at least 2000. |
| `Loss improved` | The JEPA objective improved during training. Good sign, not enough by itself. |
| `Collapse gate` | Protects against useless constant or rank-collapsed latents. Must be true for promotion. |
| `Horizon gate` | Checks multi-horizon prediction consistency. Must be true for promotion. |
| `Manifest` | JSON record for the candidate. |
| `Checkpoint` | Safetensors checkpoint for the JEPA encoders and heads. |

If training takes a while, let it finish. The default idle-trainer runtime
budget is 5 minutes; a small manual run may complete much faster.

## Step 7: Inspect The JEPA Candidate

Use the candidate id from `train-jepa`:

```bash
archon world inspect-jepa jepa-world-model-candidate-...
```

This is a read-only sanity check. It shows the model kind, latent dimension,
window sizes, loss, collapse gate, horizon gate, checkpoint path, and whether a
previous eval report passed.

## Step 8: Evaluate JEPA

Run:

```bash
archon world eval-jepa jepa-world-model-candidate-...
```

Important output:

```text
Baseline: fastembed
Baseline available: true
Relative improvement: ...
Brier regressed: false
Corpus sufficient: true
Collapse gate: true
Horizon gate: true
Checkpoint size gate: true
Tensor safety gate: true
Primary gates pass: true
```

Promotion requires `Primary gates pass: true`.

What the gates mean:

| Gate | Why it exists |
|---|---|
| `Baseline available` | Promotion compares against the local FastEmbed baseline. If FastEmbed is unavailable, JEPA fails closed. |
| `Relative improvement` | JEPA must beat the fixed generic-embedding baseline by the configured margin. |
| `Brier regressed` | Risk-label calibration must not get worse. |
| `Corpus sufficient` | JEPA needs enough examples to be meaningful. |
| `Collapse gate` | Prevents finite but useless representations. |
| `Horizon gate` | Keeps 1-step, 3-step, and 5-step prediction behavior consistent. |
| `Tensor safety gate` | Rejects NaN/Inf checkpoint values. |

If `Primary gates pass` is false, do not force it. Keep collecting traces and
try again later. `promote-jepa` will refuse a failed eval anyway.

## Step 9: Optional Baseline Comparison

This is useful for learning, not promotion:

```bash
archon world compare-representations \
  --candidate jepa-world-model-candidate-... \
  --baseline fastembed
```

You may also compare against deterministic hash for curiosity:

```bash
archon world compare-representations \
  --candidate jepa-world-model-candidate-... \
  --baseline deterministic-hash
```

That second command is exploratory only. Promotion always uses the fixed local
FastEmbed-backed baseline.

## Step 10: Promote JEPA

Only promote after `eval-jepa` says:

```text
Primary gates pass: true
```

Then run:

```bash
archon world promote-jepa jepa-world-model-candidate-...
```

This writes the active model pointer. To make runtime predictions use JEPA, set:

```toml
[learning.world_model]
model_kind = "jepa_transition"
```

Then verify:

```bash
archon world status
```

Look for:

```text
Model kind:         jepa_transition
Active model kind:  jepa_transition
JEPA status:        active
Advisor status:     ready
```

## Step 11: Use The Advisor

Ask for a prediction:

```bash
archon world predict-next \
  --session-id <session-id> \
  --action-ref verify-1 \
  --summary "run cargo test"
```

If Archon returns a prediction id, record the outcome after the action finishes:

```bash
archon world record-outcome <prediction-id> \
  --actual-summary "cargo test passed after fixing one compile error"
```

That outcome is how Archon measures surprise and improves future evaluation.

You can also score alternate actions:

```bash
archon world score-actions \
  --task "finish the JEPA implementation safely" \
  --actions actions.json
```

`score-actions` is advisory. It ranks historical similarity and risk signals; it
does not replace normal engineering judgment or verification.

## Optional: Let The Dynamic Trainer Run

The trainer is idle-aware. Defaults:

```toml
[learning.world_model.auto_trainer]
enabled = true
idle_required_ms = 300000
battery_suspend_below_percent = 30
min_throttle_ms = 3600000
```

That means:

| Setting | Meaning |
|---|---|
| `idle_required_ms = 300000` | Wait for 5 minutes without foreground activity. |
| `battery_suspend_below_percent = 30` | Avoid training on low unplugged battery. |
| `min_throttle_ms = 3600000` | Avoid training more than once per hour. |

Run one manual tick when validating the loop:

```bash
archon world trainer-tick \
  --last-activity-age-ms 600000 \
  --battery-percent 80
```

If `jepa.enabled = true`, or `model_kind = "jepa_transition"`, the tick uses the
JEPA trainer path. Otherwise it trains the default latent transition model.

## Common Outcomes

| What you see | What it means | What to do |
|---|---|---|
| `Cold-start status: cold_start (...)` | Not enough rows, sessions, or observed days. | Keep using Archon, then run `archon world ingest --backfill` again. |
| `Advisor status: fail-open` | Archon cannot safely advise yet. | This is safe. Train/promote a passing model or collect more data. |
| `Corpus sufficient: false` | JEPA has fewer than `min_training_examples`. | Keep the candidate if you want, but it cannot promote yet. |
| `Baseline available: false` | FastEmbed baseline was unavailable during JEPA eval. | Fix local embedding setup, then rerun `eval-jepa`. |
| `Collapse gate: false` | JEPA latents collapsed or lack enough rank/variance. | Collect more varied traces and train a new candidate. |
| `Brier regressed: true` | Risk-label calibration got worse than baseline. | Do not promote; collect more outcomes and retry later. |
| `jepa candidate ... has not passed all mandatory promotion gates` | Promotion is correctly refusing an unsafe candidate. | Read `eval-jepa` output, fix the failing condition, and train/eval again. |

## Reset Or Roll Back

Roll back to a previous active model:

```bash
archon world rollback <model-id>
```

Switch runtime back to the default model kind:

```toml
[learning.world_model]
model_kind = "latent_transition"
```

To reset only the world-model state, remove:

```bash
rm -rf ~/.archon/world-model
```

That deletes local world-model corpus, checkpoints, embeddings, evals, and active
pointers. It does not delete your normal Archon sessions.

## A Sensible First Week

Day 1:

```bash
archon world ingest --backfill
archon world status
```

If the status is cold, just keep using Archon.

After a few real sessions:

```bash
archon world ingest --backfill
archon world train --candidate
archon world eval <candidate-id>
archon world promote <candidate-id>
```

After the corpus is comfortably past cold start:

```bash
archon world train-jepa --candidate
archon world inspect-jepa <jepa-candidate-id>
archon world eval-jepa <jepa-candidate-id>
```

Only after `Primary gates pass: true`:

```bash
archon world promote-jepa <jepa-candidate-id>
```

Then set:

```toml
[learning.world_model]
model_kind = "jepa_transition"
```

The safest mental model is: train freely, promote slowly. Candidates are cheap.
Active advisory models should earn the pointer.
