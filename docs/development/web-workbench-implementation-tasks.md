# Web Workbench Implementation Tasks

Source PRD: `PRD-ARCHON-FINALISATION-007-web-workbench-interface.md`

Worktree: `/home/unixdude/Archon-projects/archon-cli-worktrees/web-workbench-interface`

Branch: `codex/web-workbench-interface`

Main sync: fast-forwarded to `origin/main` at `50182dd` on 2026-05-12
before commit; workspace and private web package metadata aligned to `1.2.3`.

## Guardrails

- New hand-written Rust, TypeScript, TSX, and CSS files must stay at or below
  500 lines. Generated lockfiles and generated distribution assets are exempt.
- Frontend commands should run after `source ~/.profile` so Node 22 is used.
- Cargo commands should use reduced WSL resource settings, for example:
  `prlimit --as=4294967296 -- nice -n 10 env CARGO_BUILD_JOBS=2 cargo test ...`
- Web mutating actions must use the typed action envelope and policy AND-gate.

## 007-Foundation

- [x] Create worktree from `main` under `archon-cli-worktrees`.
- [x] Replace ad-hoc web bundling with React 19 + Vite foundation.
- [x] Add strict TypeScript config and Node 22 package lock.
- [x] Add Rust-generated TypeScript DTO gate with `ts-rs`.
- [x] Add read-only API endpoints:
  `/api/status`, `/api/config/effective`, `/api/policy/effective`.
- [x] Build embedded `web/dist` bundle for `rust-embed`.
- [x] Extend FileSizeGuard to cover TypeScript, TSX, and CSS.
- [x] Add Playwright screenshot baseline for primary tabs.
- [x] Add live event manager API with cursor replay and cursor-expired recovery.
- [x] Add typed web action envelope and audit row persistence.
- [x] Add upload policy shell and attachment metadata model.
- [x] Add auth lifecycle endpoints for token/cookie posture.

## 007-DeepInspection

- [x] Add read-only DeepInspection summary adapters for corpus, learning,
  world model, pipelines, metrics, evidence, and settings.
- [x] Replace placeholder inspection tabs with API-backed summary pages.
- [x] Add bounded dark/light theme toggle with screenshot coverage.
- [x] Evidence graph: Cytoscape relationship view with node/edge budgets.
- [x] Corpus browser: rooted source list, metadata, type filters, and safe text preview.
- [x] Corpus search: bounded keyword search across source names, paths, and text previews.
- [x] Memory tab: learning signal dashboard for sessions, reasoning quality,
  calibration, and proposal stores.
- [x] World tab: persisted artifacts, advisor events, and reasoning bridge status.
- [x] Pipeline tab: stage swimlane, agent responsibilities, recent runs, and artifacts.
- [x] Metrics tab: store health, performance targets, queue depth, and event tails.
- [x] Settings tab: bounded theme editor, density toggle, and read-only policy posture.
- [x] Memory tab: row-level memories, LearningEvents, proposals, and trust deltas.
- [x] World tab: prediction, candidate, reasoning-quality, and shadow row previews.
- [x] Corpus search: ranked chunk-aware query over sources with embedding/index hints.
- [x] Memory tab: graph-style row filtering and proposal approval action previews.
- [x] World tab: promotion-gate drilldown and dry-run active checkpoint controls.
- [x] Pipeline tab: live activity stream summaries and artifact output tailing.
- [x] Metrics tab: provider latency/cost aggregation from runtime telemetry.
- [x] Settings tab: persisted server-side theme profile export/import.
- [x] Installation docs: blank-project setup and web workbench launch path.
- [x] Web workbench user guide: tabs, data sources, actions, security, troubleshooting.

## Verified So Far

- `source ~/.profile && npm run typecheck`
- `source ~/.profile && npm run build`
- `source ~/.profile && npm run test`
- `source ~/.profile && npm run test:e2e -- --update-snapshots`
- `source ~/.profile && npm run test:e2e`
- `source ~/.profile && (cd archon-sdk-ts && npm run build)`
- `prlimit --as=4294967296 -- nice -n 10 env CARGO_BUILD_JOBS=2 cargo test -p archon-sdk generated_web_api_types_match_checked_in`
- `prlimit --as=4294967296 -- nice -n 10 env CARGO_BUILD_JOBS=2 cargo test -p archon-sdk web -- --nocapture`
- `prlimit --as=4294967296 -- nice -n 10 env CARGO_BUILD_JOBS=2 cargo test -p archon-sdk --test web_ui_tests -- --nocapture`
- `prlimit --as=4294967296 -- nice -n 10 env CARGO_BUILD_JOBS=2 cargo clippy -p archon-sdk --all-targets --no-deps -- -D warnings`
- `prlimit --as=4294967296 -- nice -n 10 env CARGO_BUILD_JOBS=2 cargo test -p archon-llm fallback -- --nocapture`
- `prlimit --as=4294967296 -- nice -n 10 env CARGO_BUILD_JOBS=2 cargo test -p archon-memory embeddings -- --nocapture`
- `bash scripts/check-file-sizes.sh`
- `git diff --check`

Note: full `cargo clippy -p archon-sdk --all-targets -- -D warnings` currently
continues into path dependencies and fails on pre-existing `archon-core`
style lints unrelated to this web batch. The targeted `--no-deps` web crate
clippy gate is green.
