# Local World Model

The local world model is Archon's lightweight ME-JEPA-inspired learning layer. It does not try to be a chat model. It learns from Archon's own traces and predicts likely next state, risk, retry pressure, verification need, and plan drift.

## What It Reads

The corpus comes from existing Archon evidence:

| Source | Examples |
|---|---|
| Session activity | `~/.archon/sessions/<id>/activity/events.jsonl` |
| Audited pipelines | `.archon/pipelines/<session-id>/exports/*.jsonl` |
| Provider runtime | rate limits, fallbacks, failures, cooldowns |
| Plans | plan steps, skipped steps, drift |
| Conversations and outputs | redacted excerpts plus embedding references |
| Memory / retrospectives / agent evolution | memory surfacing, post-session learning, governed-evolution artifacts |
| Plugin artifacts | session-scoped JSON/JSONL under `.archon/plugin-artifacts`, `.archon/artifacts`, and `.archon/runs` |

Raw text is not copied into the world model by default. Rows store redacted excerpts, hashes, evidence references, labels, and embedding metadata.

## Storage

World-model state lives under `~/.archon/world-model/`:

| Path | Purpose |
|---|---|
| `world-model.db` | Cozo indexed rows and summaries |
| `ledgers/world-trace-rows.jsonl` | append-only audit ledger |
| `ledgers/world-advisor-events.jsonl` | fail-open runtime advisor events |
| `ledgers/world-runtime-outcomes.jsonl` | runtime outcomes linked to predictions |
| `ledgers/world-bundle-attachments.jsonl` | audited pipeline bundle attachments |
| `ledgers/embedding-policy-events.jsonl` | external embedding policy audit events |
| `candidates/` | candidate checkpoints |
| `active/model.json` | active advisory model pointer |

The JSONL ledger rotates at 500 MB by default and raw ledgers are retained for 90 days. Cozo summaries are retained indefinitely.

## Runtime Contract

The advisor is fail-open. When the corpus is cold, the store is unavailable, training is running, or only a candidate model exists, the advisor returns no prediction and foreground work continues.

Runtime hooks exist for shell and TUI coding/research pipelines, memory reindex, governed agent evolution, and observed provider-runtime calls. Coding/research pipelines also record pre-run counterfactual and shadow advice for alternatives such as verify-first, resume-existing, memory-surfacing, and provider fallback. Completed audited pipelines link outcomes back to persisted predictions and bundle attachment ledgers when a prediction exists.

The implementation is advisory-only. Any future behavior-changing use is gated by policy, shadow evaluation, and user approval.

## Labeling

Rows get deterministic labels first. Hybrid mode keeps those labels and adds a
provider-neutral semantic pass through the configured `LlmProvider`, so Anthropic,
Codex OAuth, and compatible providers use the same labeler path. If config or
policy denies LLM labeling, ingestion falls back to deterministic labels and
records the warning without failing the run.

## Commands

```bash
archon world status
archon world ingest <session-id>
archon world ingest --backfill
archon world train --candidate
archon world trainer-tick
archon world eval <candidate-id>
archon world promote <model-id>
archon world predict-next --session-id <id> --action-ref <ref> --summary "run tests"
archon world score-actions --task "finish feature" --actions actions.json
archon world explain <prediction-id>
archon world record-outcome <prediction-id> --actual-summary "tests passed"
archon world rollback <model-id>
```

## Hardware Backends

Default execution is CPU. Training writes JSON candidate manifests and backend-specific checkpoints. CUDA uses Candle behind the `cuda` feature. Apple Silicon Metal uses `mlx-rs` behind the `mlx-metal` feature and remains experimental until validated on real hardware. Accelerator backends are selected only after a probe creates the device, runs a tiny tensor operation, and synchronizes the result back to the host.

See [backend support](../reference/world-model-backends.md) and [embedding providers](../reference/world-model-embeddings.md).
