# World Model Handover

Branch: `codex/local-world-model`

Worktree: `/home/unixdude/Archon-projects/archon-cli-worktrees/local-world-model`

Starting commit: `4fb5884 feat: Provider Runtime Hardening & Governed Agent Evolution (PRD-006) - v1.1.0-beta.3 (#27)`

Main merge: `406b31c chore(deps): close 19 Dependabot alerts (wasmtime/jsonwebtoken/serde_yml/openssl/git2) (#31)`

Version target: `1.2.0-beta`

## Implemented

- New crate: `crates/archon-world-model`
- CLI: `archon world status`, `ingest`, `predict-next`, `record-outcome`, `score-actions`, `explain`, `train`, `trainer-tick`, `eval`, `promote`, `rollback`
- Corpus normalization for activity logs, audited traces, provider runtime events, plans, conversations, transcripts, and agent outputs
- `archon world ingest` now walks multiple session/pipeline file sources instead of only `activity/events.jsonl`
- Ingestion includes session memory, retrospective, and agent-evolution JSON artifacts when present
- JSONL audit ledger plus Cozo indexed store under `~/.archon/world-model`
- Cold-start stats, retention, active-model pointer, and fail-open advisor contract
- Runtime advisory records are appended to `~/.archon/world-model/ledgers/world-advisor-events.jsonl`; shell/TUI pipeline starts, memory reindex, governed agent-evolution commands, and observed provider-runtime starts call this path fail-open
- Compact CPU latent transition model with auxiliary heads, deterministic embedding adapter, counterfactual k-NN scoring, shadow planning, trainer gates, backend metadata, checkpoint metadata, and agent-evolution signals
- Backend selection now requires a backend probe plus tiny synchronized tensor self-test; device creation alone is not treated as accelerator availability
- Transition examples include graph-aware context features for session, agent, provider, prior plan, and prior memory-surfacing neighborhoods
- Candidate manifests and eval reports are persisted under `~/.archon/world-model/candidates`; promotion now requires a passing eval report instead of writing the active pointer blindly
- `trainer-tick` runs one idle-aware dynamic training pass with row/correction/surprise/elapsed triggers and writes a candidate checkpoint when a trigger fires
- Candidate training uses the world-model embedding config: default FastEmbed via `archon-memory`, optional config-gated OpenAI embeddings, and deterministic-hash only for tests/bootstrap
- Embeddings are redacted before provider calls by default and cached under `~/.archon/world-model/embeddings/cache`
- Provider-neutral heuristic/LLM/hybrid labeler is wired through ingestion over `LlmProvider` with config and policy gates
- `score-actions` ranks candidate actions with similarity-based k-NN over stored rows
- `predict-next` now attempts active backend candidate inference, persists a prediction record, and fails open if the active checkpoint cannot be loaded
- `record-outcome` attaches actual next-state summaries to persisted predictions and computes latent surprise
- `explain` reads persisted prediction records, including actual outcomes and latent surprise values when present
- Runtime pipeline completion updates persisted predictions, writes `world-runtime-outcomes.jsonl`, and links audited bundle metadata in `world-bundle-attachments.jsonl`
- Agent-evolution generation, shadow evaluation, approval output, and reports consume repeated world-model risk/surprise evidence
- Falsifiable eval gate calculations for next-state cosine improvement against nearest-neighbor baseline, surprise KS calibration, Brier label improvements, and all-gates promotion policy

## Verification Commands

Run under WSL with bounded resources:

```bash
prlimit --as=2147483648 -- nice -n 10 cargo fmt --all
prlimit --as=4294967296 -- nice -n 10 env CARGO_BUILD_JOBS=2 cargo test -p archon-world-model --lib
prlimit --as=6442450944 -- nice -n 10 env CARGO_BUILD_JOBS=2 cargo check --bin archon
prlimit --as=6442450944 -- nice -n 10 env CARGO_BUILD_JOBS=2 cargo test --bin archon world_model::tests
git diff --check
```

Latest local verification:

- World-model library tests pass with default features and with `--features candle`.
- `archon world_model::tests`, `archon-policy`, and single-threaded `agent_evolve` tests pass.
- `cargo check --bin archon` passes with two existing warnings in provider status/LLM helpers.
- README local-link audit and `git diff --check` pass.
- CUDA feature compilation passes with `/usr/local/cuda-13.2`, and the full CUDA-feature world-model library suite passes locally on WSL after driver/toolkit compatibility was corrected.

## Notes For Continuation

- `mlx-rs` is pinned to `=0.25.3` and only wired for macOS aarch64 behind `mlx-metal`.
- Metal is experimental until a real Apple Silicon validation run is recorded.
- Native CUDA is validated on this WSL setup; MLX Metal still requires Apple Silicon hardware validation. CPU remains the supported default.
- The first model path is intentionally small and advisory; behavior-changing use remains gated.
- No new source/doc file in this implementation should exceed 500 lines.
