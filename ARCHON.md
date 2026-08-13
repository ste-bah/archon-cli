# ARCHON.md — archon-cli Agent Configuration

Behavioural and project rules for agents operating on `ste-bah/archon-cli`.
Bias toward caution over speed. Merge per-task specifics beneath this file.

---

## 🛑 Prime Directive — Confirm Before Acting

**Overrides everything except the pipeline/agent exceptions below.**

- Present a plan, then **STOP**. Wait for explicit approval before implementing, coding, or creating files.
- Compaction / context-restore is **never** permission to resume. Summarise state, ask, wait.
- Approval = "yes / proceed / go ahead / do it / build it". NOT approval = "ok / sure / I see / makes sense" / silence / a question about the plan.
- Safe without confirmation: reading, listing, searching, status checks, answering.
- Requires confirmation: any Write/Edit, code, build/test/install, git commit/push, architecture decisions, spawning agents or workflows.

**Exceptions — the command IS the intent; execute immediately, no confirmation:**

| Command | Behaviour |
|---|---|
| `/archon-code` (50-agent) | Run the full coding pipeline uninterrupted |
| `/archon-research` (46-agent) | Run the full research pipeline uninterrupted |
| `/run-agent <name> <task>`, `/agent run` | Dispatch the named agent |

During pipeline runs do not pause for status, options, token/duration comments, or "continue?" prompts. Run the dispatch loop until the runner reports complete (or a real error). Batch mode runs all tasks back-to-back.

---

## The 12 Engineering Rules

Govern every code change, inside or outside a pipeline.

**1. Think before coding.** State assumptions; if interpretations differ, present them — don't pick silently. Surface trade-offs; push back when a simpler path exists; if unclear, stop and ask.
**2. Simplicity first.** Minimum code that solves it. No speculative features, single-use abstractions, unrequested config, or handling for impossible cases. If a senior engineer would call it overcomplicated, cut it.
**3. Surgical changes.** Touch only what the request needs. Don't reformat or refactor adjacent code, and don't delete pre-existing dead code — flag it. Only remove orphans your own change created. Every changed line traces to the request.
**4. Goal-driven execution.** Turn tasks into verifiable goals ("fix bug" → "write a failing test, then make it pass"). State a brief plan with per-step checks; loop until verified.
**5. Token budgets are real.** ~4k/task, ~30k/session. On approach, summarise and restart fresh rather than looping on the same error.
**6. Surface conflicts, don't average them.** When two patterns clash, pick one (newer / better-tested), say why, mark the other for cleanup. Never blend both.
**7. Read before you write.** Before adding code, check the crate's exports, call sites, and shared utilities (`LeannSearch` / `CartographerScan`). "Looks unrelated" is not sufficient — don't duplicate an existing function.
**8. Code decides deterministic things; the model decides judgment.** Use agents for classification, drafting, synthesis, and judgment — not for what a status code, match arm, or config already determines. (Pipeline routing is code-driven by design.)
**9. Tests verify intent, not just behaviour.** Every test must be able to fail when the business logic breaks. A test that only asserts "something returned" is decoration. Test-first (Gate 1).
**10. Checkpoint after each significant step.** State what was done, what was verified, what remains. If you can't explain current state, don't continue. Critical mid-pipeline and post-compaction.
**11. Match the codebase's conventions.** Follow existing style and structure even if you'd choose differently (snake_case, error patterns, crate layout). You may flag a bad convention — you may not quietly impose your own.
**12. Fail loud.** "Complete" is false if anything was skipped silently; "tests pass" is false if any were skipped. Surface uncertainty by default — "I couldn't verify X" beats a confident wrong "Done".

---

## Pipeline Integrity (during /archon-code & /archon-research)

First tool call MUST be `Agent("contract-agent", ...)`. Write/Edit are forbidden until Phase 4 (Implementation); Phases 1–3 are read-only. Then:

- Every agent gets a **real** subagent spawn — no fake/stub outputs, no "N/A" shortcuts, no batching multiple advances in one action. "N/A" agents (e.g. `frontend-implementer` on a backend task) still spawn; the *subagent* decides there's nothing to do, not the orchestrator.
- For each agent: read its prompt artefact, spawn, wait, write the real response to the artefact path, advance state, next.
- Self-check — if you catch "completing rapidly / streamlining the remaining agents / no work needed", STOP and say: *"INTEGRITY VIOLATION: I was about to shortcut the pipeline. Resuming correctly."*

Inventory: 50 coding agents in `crates/archon-pipeline/src/coding/agents.rs::AGENTS`; 46 research agents in `…/research/agents.rs::RESEARCH_AGENTS`. Ad-hoc agents: drop a YAML-frontmatter `.md` into `<workdir>/.archon/agents/` or `~/.config/archon/agents/`, then `/run-agent <name>`. Discover via `/agent list` or `archon agent-list`.

---

## Memory — built-in CozoDB graph (the ONLY memory system)

- `memory_store` — persist Fact/Decision/Rule/etc. `memory_recall` — hybrid BM25 + vector search.
- Never write memory to `MEMORY.md` or markdown; never call an external memory service. No MemoryGraph MCP.
- After compaction, `memory_recall "feedback corrections preferences"` to reload behavioural rules before acting.
- Graph at `~/.local/share/archon/memory.db`. `/garden` consolidates; auto-extraction ingests transcripts in the background.

---

## Search, MCP, Files

**LEANN (built in, `archon-leann`):** `LeannSearch` (semantic), `LeannFindSimilar`, `CartographerScan` (index symbols). Re-scan after major changes; pipeline runs index automatically. Not exposed via MCP.

**MCP (external only):** stdio / WebSocket / HTTP-streamable, configured in `.mcp.json` (workspace or `~/.config/archon/`). Common: `serena` (code nav), `perplexity` (web search + citations), `filesystem`, `github`, `puppeteer`, `postgres`. Memory and LEANN are built in, never MCP.

**File organisation:** never write scratch/test/working files to repo root. Use `/src`, `/crates/<crate>/src`, `/crates/<crate>/tests`, `/docs` (all user-facing `.md` here), `/scripts`, `/examples`, `/project-tasks`, `/project-work` (gitignored), `<workdir>/.archon/`. Root `.md` only for README / ARCHON / LICENSE / CHANGELOG.

---

## Code Structure & Complexity (all languages)

Applies to every language the agents produce (Rust, Python, TS/JS, Go, Java).

- Files < 500 lines preferred, **1500 hard cap** (Gate 2 / `scripts/check-file-sizes.sh`, language-agnostic). Functions < 50 lines, single responsibility. One concept per module/file (~100 lines per impl).
- Keep functions flat: cyclomatic complexity ≤ 10, nesting depth ≤ 3, ≤ 5 args. Prefer early returns / guard clauses over nested branches; extract a helper before a function grows a second concern.
- Enforce per language — Rust: `clippy::cognitive_complexity` / `too_many_lines` / `too_many_arguments`. Python: `ruff` (C901) / `radon`. JS/TS: ESLint `complexity` + `max-depth` + `max-params`. Go: `gocyclo`. The `crates/archon-tui/` ratchet (`scripts/check-tui-complexity.sh`) is the model — replicate it per language as code grows.

## Rust Idioms

- No hardcoded secrets (env vars or `~/.config/archon/`). `anyhow::Result` or typed errors — no `unwrap()`/`expect()` outside tests. No `#[allow(...)]` to mute warnings — fix the cause. Comments explain WHY.
- Separate concerns by crate; avoid circular deps. Keep `docs/` current for user-facing changes.

### Build & cargo discipline

| Task | Command |
|---|---|
| Build | `cargo build --release --bin archon` |
| Test | `cargo nextest run --workspace` |
| Check | `cargo check --workspace --tests` |
| Format / Lint | `cargo fmt --all -- --check` · `cargo clippy --workspace -- -D warnings` |

On a memory-constrained host, cap parallelism (`-j<N>`, `--test-threads=<N>`) to avoid OOM on the 21-crate workspace. Cache-corruption recovery (petgraph ICE): `cargo clean -p petgraph -p archon-pipeline && cargo build --release --bin archon`.

---

## CI Gates — `scripts/ci-gate.sh` (single source of truth)

Run locally and confirm **exit 0 before any PR or push to main**. Steps fail fast in order:

1. FileSizeGuard · 2. BannedImports · 2b. R0 entry gate (`scripts/check-r0-entry-gate.sh`) · 3. `cargo fmt --check` · 4. `cargo clippy -- -D warnings` · 5. `cargo test --workspace` · 6. baseline test-list diff · 7. `cargo bench --no-run`.

`./scripts/ci-gate.sh` (full) or `--skip-bench` (faster). TUI gates (`scripts/tui-*.sh`) run separately for `crates/archon-tui/`. The 6-gate Sherlock-narrative flow is **root archon only** — not archon-cli; don't apply it here.

---

## Truth & Audit Protocol

Subagents state only verified facts — no fallbacks/workarounds without approval, no illusions about what runs. Self-assess 1–100 vs intent; iterate to 100.

**Cold-read audit after every "COMPLETE" claim — never trust it:** re-read the diff (`git diff main..HEAD`), confirm only spec'd files changed, run the tests independently, `cargo fmt --check` + release build, confirm binary mtime/SHA matches HEAD, then approve or reject with specifics. Never blanket-approve.

---

## Core Rules (quick reference)

1. Do exactly what's asked — nothing more, nothing less; wait for explicit confirmation.
2. Never create files unless requested **and** confirmed; prefer editing existing over creating new.
3. Never create `.md`/README outside `docs/`; never write scratch/tests to repo root.
4. Post-compaction: recall behavioural rules, summarise state, ask, wait.
5. "I'll go ahead and…" is forbidden. When in doubt, ask; when not, still ask.
6. **Sequential implementation agents only** — read-only research/analysis may run in parallel.
7. `/run-agent` & `/agent run` need no confirmation — the command is the intent.
8. "Go fast" means "don't stop between tasks", NOT "skip quality gates".
9. No `Co-Authored-By:` lines in commits.

---

# Reminders

- **Prime directive:** stop and ask before acting; never auto-resume after compaction.
- **Memory:** `memory_store`/`memory_recall` only — built-in CozoDB graph, never markdown, never external.
- **Gates:** `scripts/ci-gate.sh` exit 0 before PR/push.
