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
| `ledgers/world-guardrail-actions.jsonl` | guarded interactive, tool-run, and pipeline-step actions |
| `ledgers/world-guardrail-decisions.jsonl` | guardrail policy decisions and required actions |
| `ledgers/world-guardrail-verifications.jsonl` | verification outcomes, including command exit status, manual overrides, and inconclusive results |
| `ledgers/world-guardrail-outcomes.jsonl` | final guarded-action outcomes and structured learning labels |
| `ledgers/embedding-policy-events.jsonl` | external embedding policy audit events |
| `candidates/` | candidate checkpoints |
| `jepa/candidates/` | JEPA representation candidate manifests and checkpoints |
| `jepa/evals/` | JEPA-specific promotion gate reports |
| `jepa/representation-comparisons/` | JEPA versus baseline representation comparison reports |
| `jepa/training-runs/` | JEPA component-loss training ledgers |
| `active/model.json` | active advisory model pointer |

The JSONL ledger rotates at 500 MB by default and raw ledgers are retained for 90 days. Cozo summaries are retained indefinitely.

## Runtime Contract

The advisor is fail-open. When the corpus is cold, the store is unavailable, training is running, or only a candidate model exists, the advisor returns no prediction and foreground work continues.

Runtime hooks exist for shell and TUI coding/research pipelines, memory reindex, governed agent evolution, and observed provider-runtime calls. Coding/research pipelines also record pre-run counterfactual and shadow advice for alternatives such as verify-first, resume-existing, memory-surfacing, and provider fallback. Completed audited pipelines link outcomes back to persisted predictions and bundle attachment ledgers when a prediction exists.

Runtime guardrails add a policy layer for interactive sessions, coding tasks, tool runs, verification runs, and pipeline steps. Advisory mode records risk without blocking. Guarded and strict modes gate completion records, not streamed text: required verification must pass, be explicitly skipped by a manual override, or the action remains blocked. A skipped verification satisfies a requirement only when it carries `manual_override:*` evidence. Real command/tool verification uses structured execution signals such as exit codes; LLM quality scores do not count as passed tests or builds.

Guardrail prediction remains fail-open. If prediction or guardrail storage is unavailable, Archon records the unavailable state and continues foreground work. Promotion and model selection gates remain fail-closed.

## JEPA Representation Layer

`model_kind = "jepa_transition"` enables the JEPA representation path. JEPA is a local trace-representation learner layered under the existing advisory transition model. It consumes structured trace windows, action metadata, graph context, scalar features, and deterministic lexical hashes. It does not require FastEmbed, OpenAI, or any third-party embedding provider for its own encoder path.

Training uses masked trace windows and future latent prediction. The target encoder is EMA-updated and marked stop-gradient; the predictor is a training objective only and is not invoked by runtime inference. Runtime JEPA predictions use context/action encoders plus the persisted transition model trained over JEPA latents.

JEPA promotion fails closed. A candidate must pass corpus sufficiency, collapse, multi-horizon, checkpoint-size, tensor-safety, backend-execution, and fixed FastEmbed-baseline comparison gates. If a JEPA checkpoint is missing, invalid, mismatched, slow, or cannot encode, the runtime advisor records a typed unavailable reason and foreground work continues.

## Labeling

Rows get deterministic labels first. Hybrid mode keeps those labels and adds a
provider-neutral semantic pass through the configured `LlmProvider`, so Anthropic,
Codex OAuth, and compatible providers use the same labeler path. If config or
policy denies LLM labeling, ingestion falls back to deterministic labels and
records the warning without failing the run. Backfill sends LLM labeling work in
bounded chunks (`max_events_per_prompt`, default `30`) and accepts fenced or
lightly wrapped JSON responses so provider formatting does not abort an otherwise
valid ingest. Oversized transcript rows are truncated for the labeling prompt
only, and batches split recursively until the prompt fits `max_prompt_chars`;
the persisted world-model row still keeps the configured storage/retention form.

## Commands

```bash
archon world status
archon world ingest <session-id>
archon world ingest --backfill
archon world train --candidate
archon world train-jepa --candidate
archon world trainer-tick
archon world eval <candidate-id>
archon world eval-jepa <candidate-id>
archon world inspect-jepa <candidate-id>
archon world compare-representations --baseline fastembed --candidate <candidate-id>
archon world promote <model-id>
archon world promote-jepa <candidate-id>
archon world predict-next --session-id <id> --action-ref <ref> --summary "run tests"
archon world score-actions --task "finish feature" --actions actions.json
archon world explain <prediction-id>
archon world record-outcome <prediction-id> --actual-summary "tests passed"
archon world rollback <model-id>
archon world guard status
archon world guard inspect <action-id>
archon world guard list [--session <id>] [--surface <surface>] [--status all|blocked|open|complete]
archon world guard replay-outcomes [--session <id>]
archon world guard approve <action-id> --reason "operator verified manually"
archon world guard skip-verification <action-id> --reason "not applicable"
archon world guard policy show
archon world guard policy set --interactive-mode guarded --pipeline-mode strict
```

## Hardware Backends

Default execution is CPU. Training writes JSON candidate manifests and backend-specific checkpoints. CUDA uses Candle behind the `cuda` feature. Apple Silicon Metal uses `mlx-rs` behind the `mlx-metal` feature and remains experimental until validated on real hardware. Accelerator backends are selected only after a probe creates the device, runs a tiny tensor operation, and synchronizes the result back to the host. JEPA CUDA/Metal candidates additionally require a `JepaBackendExecutionReport`; if native JEPA stages are unavailable, the candidate stays CPU-labelled with a fallback reason or the training command fails when CPU fallback is disabled.

See [backend support](../reference/world-model-backends.md) and [embedding providers](../reference/world-model-embeddings.md).
