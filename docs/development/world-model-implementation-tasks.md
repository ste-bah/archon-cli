# Local World Model Implementation Tasks

Status file for PRD-ARCHON-FINALISATION-006A.

Branch: `codex/local-world-model`
Worktree: `/home/unixdude/Archon-projects/archon-cli-worktrees/local-world-model`

Starting point:

- Source branch: `origin/main`
- Starting commit: `4fb5884 feat: Provider Runtime Hardening & Governed Agent Evolution (PRD-006) - v1.1.0-beta.3 (#27)`
- Main merge: `406b31c chore(deps): close 19 Dependabot alerts (wasmtime/jsonwebtoken/serde_yml/openssl/git2) (#31)`
- Version target: `1.2.0`

Build hygiene:

- Run Cargo with WSL limits: `prlimit`, `nice`, and `CARGO_BUILD_JOBS=2`.
- Do not use unbounded full-workspace build loops.
- Do not add new source or doc files over 500 lines.

Status legend:

- Complete: implemented, wired where required, and covered by tests/docs.
- Partial: useful code exists, but it is scaffold/baseline behavior or is not wired end to end.
- Not started: no production implementation yet.

## Milestone Summary

| Milestone | Real Status | Notes |
|---|---|---|
| M1: Corpus and CPU latent model core | Complete | Schema, storage, ingestion adapters, plugin artifact discovery, cold-start, labels, real embeddings, compact CPU latent model, graph context, eval math, Candle safetensors, and MLX array checkpoint artifacts are implemented. |
| M2: Runtime advisor and retention | Complete | Fail-open API, runtime advisory ledger, shell/TUI pipeline hooks, memory CLI hook, agent-evolution hook, provider-runtime hook, runtime outcome linkage, audited bundle attachment, status calibration, and retention exist. |
| M3: Counterfactual and shadow planning | Complete | k-NN scoring, shadow ranking, prediction evidence, runtime counterfactual advice, persisted score records, and pipeline pre-run shadow advice are wired. |
| M4: Dynamic trainer and accelerators | Complete; Apple Silicon validation pending | Idle/battery trigger logic, background pipeline tick, CPU fallback, accelerator cap checks, backend metadata, backend probe/self-test, Candle CPU/CUDA paths, MLX Metal paths, Candle safetensors, and MLX checkpoint artifacts exist. CUDA feature validation passes locally on WSL; Apple Silicon validation remains an external gate. |
| M5: Agent evolution and docs finalisation | Complete | Agent-evolution proposals, shadow evidence, approval output, and reports consume world-model signals. README-linked docs, config, policy, CLI, slash, backend, embedding, release-note, and runtime docs are updated. |

## Setup

- [x] Create worktree from `origin/main`.
- [x] Create branch `codex/local-world-model`.
- [x] Record starting commit.
- [x] Pull in main-branch Dependabot fix commit `406b31c`.
- [x] Bump workspace version to `1.2.0`.
- [x] Add handover note with starting commit and implementation scope.

## M1: Corpus And Latent Model Core

Complete:

- [x] Add `crates/archon-world-model` to the workspace.
- [x] Add config structs for `[learning.world_model]`, embeddings, training, eval, cold start, auto-trainer, and retention.
- [x] Add normalized `WorldTraceRow` data types with evidence references.
- [x] Add JSONL and Cozo world-model storage.
- [x] Add cold-start threshold calculation: `1000` rows, `50` sessions, `7` observed days by default.
- [x] Add activity JSONL, provider runtime JSONL, plan JSON, conversation, transcript, and agent-output normalization helpers.
- [x] Add session/backfill file walker for activity logs, subagent transcripts, plans, provider-runtime JSONL, transcript/output JSON, and pipeline export JSONL.
- [x] Add first-class file discovery and normalization for session memory, retrospectives, and agent-evolution artifacts.
- [x] Add deterministic label builder for basic failure/retry/provider/verification/correction/plan-drift labels.
- [x] Add embedding adapter trait and deterministic local hash adapter for tests/bootstrap.
- [x] Add runtime local FastEmbed adapter through `archon-memory`.
- [x] Add config-gated OpenAI third-party embedding adapter with projection to world-model state dimension.
- [x] Add projection metadata.
- [x] Add persistent embedding cache keys and redaction-before-embedding enforcement.
- [x] Add provider-neutral heuristic/LLM/hybrid semantic labeler over `LlmProvider`.
- [x] Add eval calculations for next-state nearest-neighbor baseline, surprise KS, Brier improvement, and promotion gate composition.
- [x] Add stored-row to transition-example builder for candidate training.
- [x] Add CPU candidate manifest save/load and eval report save/load.
- [x] Write/read Candle safetensors checkpoint files for CPU candidates.
- [x] Replace mean-delta-only baseline with compact CPU latent transition weights and auxiliary label heads.
- [x] Feed auxiliary-head Brier scores into candidate eval reports.
- [x] Add graph-aware context features for session, agent, provider, plan, and memory neighborhoods.
- [x] Add repository-independent plugin artifact discovery under `.archon/plugin-artifacts`, `.archon/artifacts`, and `.archon/runs`.
- [x] Add MLX array checkpoint save/load artifact support.
- [x] Add policy-ledger audit events for external embedding calls.
- [x] Wire LLM/hybrid labeling into ingestion with config and policy gates.
- [x] Include prior plan and memory evidence IDs in graph-aware features.
- [x] Add unit tests for schema, labels, cold-start, storage, ingestion, eval gates, and bridge metadata.

## M2: Runtime Advisor And Retention

Complete:

- [x] Add `WorldAdvisor` API returning `Option<WorldPrediction>`.
- [x] Emit typed `WorldAdvisorUnavailable` events in the library API.
- [x] Add JSONL rotation and raw-retention helpers.
- [x] Add Cozo summary retention behavior.
- [x] Add `archon world status`, `predict-next`, `train`, `eval`, `promote`, and `rollback` command surfaces.
- [x] Expand `archon world status` with active model, candidate count, backend fallback, trainer, and advisor state.
- [x] Wire `archon world train` to persisted rows and backend candidate manifests.
- [x] Wire `archon world eval` to candidate manifests and persisted eval reports.
- [x] Block `archon world promote` unless a candidate eval report passes every mandatory gate.
- [x] Persist model activation history with previous pointer metadata for promote/rollback.
- [x] Wire `archon world predict-next` to active backend candidate inference when an active candidate manifest exists.
- [x] Persist successful `predict-next` records and allow `archon world explain <prediction-id>` to read them back.
- [x] Add `archon world record-outcome <prediction-id> --actual-summary <text>` to attach actual next state and compute latent surprise.
- [x] Persist typed runtime advisory surface records to `ledgers/world-advisor-events.jsonl`.
- [x] Add non-blocking world-model advisory hooks to shell coding/research pipeline starts.
- [x] Add non-blocking world-model advisory hooks to TUI `/archon-code` and `/archon-research` starts.
- [x] Add non-blocking world-model advisory hook to the memory reindex foreground path.
- [x] Add non-blocking world-model advisory hook to the governed agent-evolution shell surface.
- [x] Add last-eval calibration summary to `archon world status`.
- [x] Persist runtime predictions from active checkpoints and attach pipeline outcomes automatically.
- [x] Persist runtime outcome and audited bundle attachment ledgers.
- [x] Compute latent surprise automatically for runtime outcomes when a persisted prediction exists.
- [x] Feed the counterfactual gate from observed heldout labels instead of leaving it as a placeholder.
- [x] Write promotion validation metadata beside the active model pointer.
- [x] Add non-blocking advisor calls to lower-level observed provider-runtime foreground flows.
- [x] Add advisor fail-open and retention tests.

## M3: Counterfactual And Shadow Planning

Complete:

- [x] Add k-NN counterfactual scoring over historical action embeddings.
- [x] Report counterfactual calibration separately from observed-action calibration.
- [x] Add shadow action ranking and NDCG helpers.
- [x] Add `archon world score-actions` shell surface using k-NN over stored rows.
- [x] Add `archon world explain` shell surface with persisted prediction, outcome, and surprise state.
- [x] Persist counterfactual score records with neighbor evidence refs.
- [x] Add evidence refs to persisted predictions and explain output.
- [x] Score real runtime alternatives for provider fallback, verification, resume, and memory surfacing before coding/research pipelines.
- [x] Store runtime outcome records once pipeline choices complete.
- [x] Wire counterfactual advice before coding/research pipeline starts.
- [x] Wire shadow planning before expensive coding/research pipeline paths.
- [x] Add counterfactual and shadow planning tests.

## M4: Dynamic Trainer And Accelerators

Complete:

- [x] Add idle-aware and low-battery trainer decision helpers.
- [x] Add trigger policy for new rows, surprises, corrections, elapsed time, and first run.
- [x] Add one-shot dynamic trainer tick that consumes stored rows and writes candidate checkpoints when gated triggers pass.
- [x] Add backend status metadata for CPU, CUDA, Metal, and auto.
- [x] Add feature-gated Candle CPU/CUDA and MLX Metal forward paths for the compact transition model.
- [x] Add Candle/MLX bridge metadata and tests for dtype, shape, NaN/Inf, memory order, and fp32 parity threshold.
- [x] Add backend-specific checkpoint format metadata.
- [x] Add backend-native Candle safetensors checkpoint roundtrip.
- [x] Add backend-native MLX array checkpoint roundtrip artifact support.
- [x] Wire dynamic trainer ticks into shell/TUI pipeline completion as a background task.
- [x] Enforce accelerator memory caps when a non-CPU backend is selected.
- [x] Implement explicit CPU fallback behavior for training and trainer-tick paths.
- [x] Pin `mlx-rs` behind the `mlx-metal` feature.
- [x] Select runtime backend from compiled feature/hardware availability instead of hardcoded CPU-only status.
- [x] Probe every accelerator backend with a tiny synchronized tensor self-test before selecting or training on it.
- [x] Route candidate training and active prediction through the selected backend with backend-specific checkpoint format.

- Native CUDA tensor training/inference code is feature-gated and has passed local WSL validation with the driver-compatible CUDA 13.1 toolkit path. The validation record lives in `docs/development/world-model-cuda-validation.md`; the CUDA 13.2 toolkit is installed but this driver rejects its generated PTX.
- Native MLX Metal tensor training/inference code is feature-gated and needs Apple Silicon validation before removing experimental status. The pending validation checklist lives in `docs/development/world-model-mlx-metal-validation.md`.
- Record real Apple Silicon validation before changing Metal from experimental to supported.

## M5: Agent Evolution And Documentation Finalisation

Complete:

- [x] Add world-model docs, release notes, backend matrix, embedding matrix, and dynamic-training cookbook.
- [x] Update README, config docs, policy docs, CLI docs, slash-command docs, and generated command-surface docs.
- [x] Add library helper for repeated world-model risk/surprise signals.
- [x] Wire world-model signals into governed agent-evolution proposal, shadow, approval, and report flows.

- [x] Update `docs/architecture/learning-systems.md` with the world-model learning loop.
- [x] Run README local-link audit.
- [x] Update runtime documentation after pipeline/provider/memory integrations are real.

## Remaining External Validation Gates

- Run MLX Metal validation on real Apple Silicon before removing the experimental status; update `docs/development/world-model-mlx-metal-validation.md` with device, command, candidate, and execution-report evidence.
- Keep CUDA validation in CI/manual release notes when CUDA runners are available; local WSL validation now passes.

## Verification Results

- `cargo test -p archon-world-model --lib`: passed, 75 tests.
- `cargo test -p archon-world-model --lib --features candle`: passed, 75 tests.
- `cargo check -p archon-world-model --features candle --lib`: passed.
- `cargo check -p archon-world-model --features mlx-metal --lib`: passed on Linux target with non-Apple MLX stubs.
- `cargo test --bin archon world_model::tests`: passed, 15 tests.
- `cargo test -p archon-policy`: passed, 14 tests.
- `cargo test --bin archon agent_evolve -- --test-threads=1`: passed, 38 tests.
- `cargo check --bin archon`: passed with two pre-existing warnings in provider status/LLM helpers.
- README local-link audit: passed.
- `git diff --check`: passed.
- `cargo check -p archon-world-model --features cuda --lib`: passed with explicit `/usr/local/cuda-13.1` toolkit environment.
- `cargo test -p archon-world-model --lib --features cuda candle_cuda_trains_and_predicts_when_available -- --nocapture`: passed locally on WSL after driver/toolkit compatibility was corrected.
- `cargo test -p archon-world-model --lib --features cuda`: passed, 127 tests plus 7 ignored hardware tests.
- `cargo test -p archon-world-model --features cuda --lib jepa_cuda -- --ignored --nocapture --test-threads=1`: passed, 7 hardware tests.
- `cargo test --bin archon --features cuda world_model::tests::predict_next_uses_active_jepa_cuda_model -- --ignored --nocapture --test-threads=1`: passed, 1 hardware test.
