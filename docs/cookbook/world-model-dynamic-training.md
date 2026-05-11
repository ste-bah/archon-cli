# World Model Dynamic Training

Use the world model to let Archon learn from its own traces without blocking the current session.

## Backfill The Corpus

```bash
archon world ingest --backfill
archon world status
archon world train --candidate
archon world eval <candidate-id>
```

The advisor stays in cold start until the configured thresholds are met:

```toml
[learning.world_model.cold_start]
min_rows = 1000
min_sessions = 50
min_observed_days = 7
```

## Keep Training Consumer Friendly

Training is idle-aware:

```toml
[learning.world_model.auto_trainer]
enabled = true
idle_required_ms = 300000
battery_suspend_below_percent = 30
max_runtime_ms = 300000
```

That means training waits until recent foreground activity has been quiet for five minutes, suspends on low unplugged battery, and caps each run at five minutes.

Candidate training writes a checkpoint manifest first. Promotion is blocked until
`archon world eval <candidate-id>` records an eval report where every mandatory
gate passes.

Shell and TUI coding/research pipelines schedule a non-blocking trainer tick
after completion. If the idle, battery, row-count, surprise, correction, and
elapsed-time gates do not allow training, the tick exits quietly.

## Choose A Backend

```toml
[learning.world_model.training]
backend = "auto"
allow_cpu_fallback = true
max_accelerator_memory_mb = 4096
```

Use CPU as the reliable baseline. CUDA and Metal are optional feature-gated accelerator paths. Archon probes accelerators with a tiny synchronized tensor operation before training; failed probes fall back to CPU when `allow_cpu_fallback = true`. Metal is Apple Silicon only and remains experimental until validated on real hardware.

Run one idle-aware trainer tick manually when validating the loop:

```bash
archon world trainer-tick --last-activity-age-ms 600000 --battery-percent 80
```

## Advisory Use

```bash
archon world predict-next --session-id <id> --action-ref verify-1 --summary "run cargo test"
archon world record-outcome <prediction-id> --actual-summary "cargo test passed after retry"
archon world score-actions --task "finish feature" --actions actions.json
```

If the model is cold, training, missing an active checkpoint, or unavailable, Archon reports the reason and continues the foreground task.

Record outcomes with redacted summaries after the action completes. That is how the world model computes surprise from predicted-vs-actual next state.

`score-actions` is similarity-based rather than causal. Use it as advisory
ranking input, then verify the chosen path normally.

The shell and TUI pipeline paths record this advisory lifecycle automatically:
pre-run prediction, pre-run counterfactual alternatives, completion outcome,
surprise calculation, and audited bundle attachment when a bundle exists.
