# Reasoning Quality Implementation Tasks

Status file for PRD-ARCHON-FINALISATION-006C.

Branch: `codex/reasoning-quality-events`
Worktree: `/home/unixdude/Archon-projects/archon-cli-worktrees/reasoning-quality-events`

Starting point:

- Source branch: `origin/main`
- Starting commit: `075faf4 feat: implement 3 deferred bench bodies + apply_diff consistency check [skip ci]`
- Version target: `1.2.0-beta`

Build hygiene:

- Run Cargo with WSL limits: `prlimit`, `nice`, and `CARGO_BUILD_JOBS=2`.
- Do not use `timeout`.
- Do not add new source or doc files over 500 lines.

Status legend:

- Complete: implemented, wired where required, and covered by tests/docs.
- Partial: useful code exists, but it is not wired end to end.
- Not started: no production implementation yet.

## Milestone Summary

| Milestone | Real Status | Notes |
|---|---|---|
| M1: Core crate, store, extractor, fixture gates | Complete | Added `archon-reasoning-quality`, typed events, JSONL/Cozo store, deterministic extractor, canonical IDs, redaction, severity, audit, and 150-row fixture gates. |
| M2: Agent emission and chronology | Complete | Visible assistant turns, tool evidence, memory/chat/prior-claim/pipeline evidence, correction linking, superseding source rows, and simulated-session tests are wired. |
| M3: Bridges | Complete | JSONL source-of-truth, LearningEvent/world-model bridges, shadow deltas, shadow-window exit, active trust updates, bridge offsets, dead-letter writes, replay for new-format dead letters, briefing summaries, and canonical e2e test exist. |
| M4: LLM critic and budgets | Complete | Provider data-flow classification, policy gates, async active-provider critic calls, parser, coverage state, budget ledger, budget tests, and fake-provider runtime test exist. |
| M5: Proactive briefing and patterns | Complete | Session-start briefing, pre-message cwd/branch context, first-turn task-context update event, pending proposal/world-model/reasoning sections, slash preview, repeated-pattern detection, shadow-hold nudge, and sample-label flow exist. |
| M6: Backfill, migrations, docs, command surface | Complete | CLI commands, config/policy examples, slash mirrors, fixture audit, deterministic session backfill, migration records, dead-letter replay, docs/references, release notes, and command-surface tests are done. |

## Setup

- [x] Remove old local world-model worktree.
- [x] Create worktree from `origin/main`.
- [x] Create branch `codex/reasoning-quality-events`.
- [x] Record starting commit.
- [x] Confirm workspace version is `1.2.0-beta`.
- [x] Keep this task list updated after each implementation slice.

## M1: Core Crate, Store, Extractor, Fixture Gates

- [x] Add `crates/archon-reasoning-quality` to the workspace.
- [x] Define `ReasoningQualityEvent`, claim, evidence, bridge, shadow, critic-cost, and schema-migration types.
- [x] Implement `canonicalize_claim_text()` with `canonicalizer_version` and deterministic `claim_id`.
- [x] Redact entity keys, excerpts, home/workspace paths, URLs, provider/auth ids, and secret-like values.
- [x] Implement deterministic severity table, confidence modifiers, and audited override validation.
- [x] Implement deterministic claim extraction for codebase, tests, completion, provider, config, docs, plan, and general reasoning.
- [x] Cover edge cases: negation, conditionals, multi-turn supersession, re-assertion, quoted text, and code fences.
- [x] Implement entity extraction for paths, tests, config keys, provider ids, plan ids, and general entities.
- [x] Implement JSONL append store under `~/.archon/reasoning-quality/events/YYYY-MM-DD.jsonl`.
- [x] Implement Cozo schema and upserts for all PRD relations.
- [x] Implement extractor fixture evaluator and quality gates.
- [x] Add at least 150 labeled turn fixtures with redacted real/synthetic examples.
- [x] Add fixture-audit library logic for secrets, email/token-like strings, high-entropy values, and absolute user paths.
- [x] Add M1 unit tests and keep every new file under 500 lines.

## M2: Agent Emission And Chronology

- [x] Add agent-core hooks for visible assistant turns, evidence/tool chronology, and user corrections.
- [x] Emit `claim_before_source_read`, `test_status_claim_without_command`, and `completion_claim_without_evidence`.
- [x] Link user corrections to the most likely prior claim.
- [x] Emit superseding `source_verified_claim` and `claim_contradicted_by_source` rows without mutating originals.
- [x] Implement relevance matching with `verified_before_claim`, `partially_verified`, `unverified`, and `needs_human_review`.
- [x] Treat memory, chat history, prior verified claims, MCP/plugin results, and pipeline artifacts as valid evidence sources.
- [x] Add simulated-session and chronology fixture tests.

## M3: Bridges

- [x] Implement source-of-truth event store plus best-effort bridge consumers.
- [x] Add bridge idempotency keys and offsets.
- [x] Add dead-letter ledger, status reporting, and replay support for new-format dead letters with embedded event JSON.
- [x] Bridge reasoning events to `LearningEvent` rows.
- [x] Bridge reasoning events to world-model rows while capping cold-start contribution at 25%.
- [x] Bridge self-trust changes through atomic counter increments, shadow deltas, and shadow-window exit.
- [x] Bridge briefing summaries for startup ranking.
- [x] Add canonical e2e test: claim, correction, LearningEvent, world-model row, self-trust delta, briefing candidate.
- [x] Prevent retrospective duplicate trust-affecting LearningEvents for reasoning-quality-covered sessions.

## M4: LLM Critic And Budgets

- [x] Add optional critic API over `LlmProvider`.
- [x] Enforce config and policy gates for `allow_llm_critic`, third-party critics, cloud data flow, and raw text.
- [x] Add `DataFlowClass` classification: `Local`, `Cloud`, `UserOperated`.
- [x] Add critic JSON parser with safe failure to `critic_unavailable`.
- [x] Enforce per-session token caps and daily/weekly USD caps.
- [x] Process critic claims in severity-first order and record partial/none/full coverage.
- [x] Add `archon reasoning cost status`.
- [x] Add fake-provider, policy, budget, and parser tests.

## M5: Proactive Briefing And Patterns

- [x] Compose memory briefing, reasoning warnings, pending behaviour proposals, and world-model advisory.
- [x] Respect policy-disabled briefing injection.
- [x] Rank briefing items by severity, recency decay, and current-task relevance.
- [x] Support pre-message fallback from cwd, branch, git HEAD, and recent activity.
- [x] Emit `briefing_updated_with_task_context` after the first user message.
- [x] Add repeated-pattern detection across 30 days, 3 events, and 3 sessions.
- [x] Keep repeated patterns shadow-only until post-shadow revalidation passes.
- [x] Surface shadow-hold operator nudges with exact next commands.
- [x] Add briefing and repeated-pattern tests.

## M6: Backfill, Migrations, Docs, Command Surface

- [x] Add `archon reasoning status`.
- [x] Add `archon reasoning inspect <session-id>`.
- [x] Add `archon reasoning backfill [--sessions N] [--emit-world-rows] [--include-llm]`.
- [x] Add `archon reasoning claims <session-id>`.
- [x] Add `archon reasoning patterns`.
- [x] Add `archon reasoning replay-dead-letter [--bridge <name>]`.
- [x] Add `archon reasoning shadow-report`.
- [x] Add `archon reasoning sample-label <session-id> [--turn <N>]`.
- [x] Add `archon reasoning migrate --to-version <N> [--dry-run]`. Current implementation records append-safe schema migration metadata.
- [x] Add `archon reasoning fixture-audit`.
- [x] Add `archon briefing preview [--task "..."]`.
- [x] Add slash commands: `/reasoning status`, `/reasoning inspect`, `/reasoning patterns`, `/briefing preview`.
- [x] Add config examples and structs for `[learning.reasoning_quality]`, critic, budgets, extractor eval, patterns, and session briefing.
- [x] Add policy examples and structs for `[policy.reasoning_quality]`.
- [x] Update README, config docs, policy docs, slash docs, command-surface docs, release notes, and generated references.
- [x] Add `docs/architecture/reasoning-quality.md`.
- [x] Add `docs/architecture/learning-systems-index.md`.
- [x] Add `docs/cookbook/proactive-session-briefing.md`.
- [x] Run docs link audit, command-surface tests, fixture gates, and focused cargo tests. Link audit is manual/new-link scoped; no repository script exists.
