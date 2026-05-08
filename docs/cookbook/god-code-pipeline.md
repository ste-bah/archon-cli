# Running the coding pipeline (`/archon-code`)

End-to-end walkthrough of the 50-agent coding pipeline. The TUI primary is `/archon-code` — equivalent to the shell command `archon pipeline code <task>`. Both forms drive the same pipeline machinery; the slash form just runs through the in-session command dispatcher.

> **TUI parity.** Every `archon X` shell command in this doc has a `/X` slash equivalent inside the interactive TUI. See [CLI and TUI Command Parity](real-world-evidence-engine.md#cli-and-tui-command-parity).

> **Provider parity.** The pipeline uses the active provider. Anthropic
> OAuth/API-key/proxy remains the default; set `[llm].provider =
> "openai-codex"` after `archon auth login --provider openai-codex` to run the
> same coding workflow through Codex. Activity rows show provider/model/cost
> where the backend reports it.

## When to use

The pipeline shines on:
- New features that span multiple files / crates
- Refactors with cross-cutting concerns
- Implementations that need design + tests + review
- Tasks where you want every change reviewed by a specialized agent

For one-off edits, just chat normally — the pipeline overhead isn't worth it for trivial changes.

## Evidence-Aware Coding Example

Use the coding pipeline when the implementation must leave proof behind:

```bash
archon pipeline code \
  "Add archon docs summarize <document-id>. It must read persisted chunks, produce cited summaries, write answer provenance edges, add tests, and update docs." \
  --dry-run

archon pipeline code \
  "Add archon docs summarize <document-id>. It must read persisted chunks, produce cited summaries, write answer provenance edges, add tests, and update docs." \
  --max-budget-usd 20
```

After it finishes, inspect the claims instead of trusting the final paragraph:

```bash
archon pipeline verify <session-id> --write-report
archon pipeline inspect <session-id>
archon pipeline export-traces <session-id> --out traces.jsonl
archon completion verify <run-id> --agent code-quality-improver --model sonnet
archon completion incidents
archon completion trust --agent code-quality-improver
```

If the run creates learning events, review them:

```bash
archon behaviour status
archon behaviour generate-proposals
archon behaviour list-proposals
```

## Trigger

Inside the TUI (recommended):

```
> /archon-code implement OAuth2 token refresh with file locking
Starting coding pipeline for task: implement OAuth2 token refresh with file locking
[task-analyzer] parsing task contract...
[task-analyzer] complete (2.1s, $0.04)
[requirement-extractor] extracting functional + non-functional requirements...
…
```

The handler spawns the audited pipeline async via `tokio::spawn`. Per-agent progress streams through canonical activity events and conversation output, while prompts, attempts, accepted outputs, quality scores, and state are persisted under `<workdir>/.archon/pipelines/<session-id>/`. The conversation stays interactive — keep using other slash commands while the run is in flight.

Equivalent CLI invocation (same persisted state, same outputs):

```bash
archon pipeline code "implement OAuth2 token refresh with file locking"

# Dry run (plan without executing) — only available via the CLI form
archon pipeline code "..." --dry-run
```

The CLI form supports `--dry-run` and `--max-budget-usd` flags. The slash form takes the task as positional arguments only — set the budget cap via `.archon/policy.toml`, or use the CLI form when you need per-run overrides.

## What happens

The pipeline runs 6 phases sequentially. Each phase has reviewers that gate progression to the next.

### Phase 1: Understanding (8 agents)

`contract-agent` parses the input contract → `requirement-extractor` pulls out functional/non-functional requirements → `requirement-prioritizer` MoSCoW-orders them → `scope-definer` sets boundaries → `context-gatherer` reads existing code → `feasibility-analyzer` validates technical feasibility → `pattern-explorer` identifies relevant patterns → `technology-scout` evaluates external solutions.

Output: agent output persisted in the audited bundle with prompt hashes, token/cost metadata, and quality scoring.

### Phase 2: Exploration (5 agents)

`context-gatherer` reads existing code → `codebase-analyzer` maps architecture → `pattern-explorer` identifies relevant patterns → `technology-scout` evaluates external solutions → `ambiguity-clarifier` resolves unknowns.

Output: exploration output persisted in the audited bundle with prompt and attempt records.

### Phase 3: Architecture (7 agents)

`system-designer` does high-level → `component-designer` does internal → `interface-designer` defines APIs → `data-architect` designs storage → `security-architect` flags threats → `integration-architect` plans external connections → `performance-architect` plans for load.

Output: architecture output persisted in the audited bundle.

### Phase 4: Implementation (12 agents)

Splits the work: `code-generator`, `unit-implementer`, `service-implementer`, `api-implementer`, `frontend-implementer`, `data-layer-implementer`, `type-implementer`, `error-handler-implementer`, `logger-implementer`, `config-implementer`, `integration-tester`, `dependency-manager`.

Each writes its slice in parallel where possible. Output: actual code in `<workdir>` plus audited agent outputs and retry attempts in the bundle.

### Phase 5: Quality (7 agents)

`code-quality-improver`, `sherlock-holmes` (forensic review), `security-tester`, `regression-tester`, `coverage-analyzer`, `code-simplifier`, `final-refactorer`. The Sherlock Holmes agent independently re-reads the code; reviews from other agents are not trusted.

Output: quality findings, test evidence, retry decisions, and accepted outputs in the audited bundle.

### Phase 6: Sign-off (8 agents)

`sign-off-approver` plus phase-1 through phase-6 reviewers. Each phase is checked once more. Final approval gates the pipeline closing.

Output: final sign-off output. Pipeline marks the session complete, runs completion integrity on the final answer in the CLI path, and stores the summary in bundle state.

## Monitoring progress

```
# In another terminal
archon pipeline status <session-id>
archon pipeline list
archon pipeline verify <session-id>
archon pipeline inspect <session-id>
```

The TUI shows live progress with phase indicators.

## Resuming

If archon-cli crashes or you `Ctrl-C`:
```bash
archon pipeline list                      # find your session
archon pipeline verify <session-id>       # optional preflight
archon pipeline resume <session-id>       # verifies bundle, then continues
```

Resume requires git working tree consistency. It also verifies the audited bundle before continuing so corrupted state, missing outputs, or mismatched hashes fail closed.

## Aborting

```bash
archon pipeline abort <session-id>
```

Marks the bundle aborted and preserves manifest, state, audit log, prompts, outputs, and attempt records for forensic review.

## Cost expectations

Full 50-agent pipeline on a moderate task (e.g., new feature spanning 3 crates):
- ~150-300k input tokens (heavy due to L0-L3 layered context)
- ~20-50k output tokens
- Sonnet 4.6: $5-15
- Opus 4.7 (heavy phases only): $15-40

Set a hard limit:
```bash
archon pipeline code "..." --max-budget-usd 20
```

## Customizing

The pipeline's canonical agent list is `crates/archon-pipeline/src/coding/agents.rs::AGENTS`; each entry points at a prompt file under `.archon/agents/coding-pipeline/`. Override a prompt per project with the same path:

```
<workdir>/.archon/agents/coding-pipeline/code-quality-improver.md
```

A project-local agent definition takes precedence over the built-in.

## Dev flow gates (separate concept)

Don't confuse the pipeline's deterministic gates (between phases) with archon-cli's CI gates (`scripts/ci-gate.sh`). The pipeline gates govern phase transitions during a `/archon-code` run; the CI gates govern code quality before merge. Different concerns.

See [CI gates](../development/dev-flow-gates.md) for the technical CI flow (file-size, banned-imports, fmt, clippy, test, baseline diff, bench compile-check).

## End-to-end TUI walkthrough

What driving a coding-pipeline run from inside the TUI actually looks like. Assumes you're at the `archon` prompt and authenticated.

### Discover-and-plan loop (recommended)

Always dry-run first. The plan is cheap (no LLM cost) and tells you whether the pipeline understood your task before you spend $5-15 on a real run. The dry-run output is only available through the CLI form — use it from a second terminal:

```bash
$ archon pipeline code "Add archon docs summarize <doc-id>: read persisted chunks, produce cited summaries, write provenance edges, add tests, update docs" --dry-run
=== Coding Pipeline Dry Run ===
Task: Add archon docs summarize <doc-id>: read persisted chunks, produce cited
      summaries, write provenance edges, add tests, update docs

Agent Sequence (50 agents):
  Phase 1: task-analyzer, requirement-extractor, requirement-prioritizer
  Phase 2: pattern-explorer, technology-scout, feasibility-analyzer, codebase-analyzer
  Phase 3: system-designer, component-designer, interface-designer, ...
  Phase 4: code-generator, unit-implementer, api-implementer, ...
  Phase 5: test-generator, integration-tester, security-tester, ...
  Phase 6: final-refactorer, sign-off-approver

Estimated cost: ~$2.50-5.00 (varies by task complexity)
```

Then drive the actual run from inside the TUI:

```
> /archon-code Add archon docs summarize <doc-id>: read persisted chunks, produce cited summaries, write provenance edges, add tests, update docs
Starting coding pipeline for task: Add archon docs summarize <doc-id>: ...
[task-analyzer] parsing task contract...
[task-analyzer] complete (2.1s, $0.04)
[requirement-extractor] extracting functional + non-functional requirements...
[requirement-extractor] complete (3.8s, $0.07)
[requirement-prioritizer] MoSCoW-ordering 14 requirements...
…
```

### Live progress in the TUI

The Agent Activity rail shows the parent turn plus active subagent rows live,
including provider/model/cost metadata where known:

```
─── Agent Activity ─────────────────────────────────────────────
  ▶ pipeline-coordinator   openai-codex/gpt-5.4 running   00:42
    └─ [AGENT] task-analyzer       openai-codex/gpt-5.4 done       3.1s
    └─ [AGENT] requirement-extractor             done       4.8s
    └─ [AGENT] requirement-prioritizer           running    1.2s
─────────────────────────────────────────────────────────────────
```

The rail derives rows from canonical activity events; every spawned subagent
appears as an `[AGENT]` row that moves `running → done | failed`.

### Status — from the same TUI session

You don't need a second terminal. The slash form runs through the same dispatcher and queries the same persisted store:

```
> /pipeline list
SESSION ID                                 KIND    PHASE       STATUS    STARTED
01HYCD3WSXKJ8R…                            coding  phase-3     running   2026-05-04 19:12
01HYCD0GMQ1YZP…                            coding  phase-6     complete  2026-05-04 18:01

> /pipeline status 01HYCD3WSXKJ8R…
Status:    InProgress (phase 3 of 6)
Phase:     architecture
Last agent: component-designer (completed 8s ago)
Cost:      $4.31 / $20.00 budget
Started:   2026-05-04 19:12:33Z
Resumeable: yes
```

### Resume after a crash

If archon dies mid-pipeline (Ctrl-C, OOM, network blip), restart it and pick up where you left off — entirely from the TUI:

```
$ archon
> /pipeline list
> /pipeline resume 01HYCD3WSXKJ8R…
[recovery] verifying git working tree...
[recovery] last completed gate: phase-2 sign-off-approver
[recovery] resuming at phase-3 system-designer
```

The recovery layer verifies the audited bundle and refuses to resume if the persisted records no longer match their hashes. Commit or stash unrelated work before resuming if git-state checks fire.

### Abort cleanly

```
> /pipeline abort 01HYCD3WSXKJ8R…
[abort] killing in-flight subagents...
[abort] bundle marked aborted at .archon/pipelines/01HYCD3WSXKJ8R…
```

Forensic-review-friendly: the bundle stays so you can reconstruct prompts, outputs, retries, quality scores, and the final completion check.

### Inspecting after completion

```
> /pipeline status 01HYCD0GMQ1YZP…
Status:    Complete
Phase:     phase-6 sign-off
Total cost: $11.48
Agents run: 48 / 48
Files modified: crates/archon-docs/src/summarize.rs (new), tests/docs_summarize_smoke.rs (new), 4 others
```

Then verify the claims from inside the TUI rather than trust the final paragraph:

```
> /pipeline verify 01HYCD0GMQ1YZP… --write-report
> /pipeline inspect 01HYCD0GMQ1YZP…
> /completion verify 01HYCD0GMQ1YZP… --agent code-quality-improver --model sonnet
> /completion incidents
> /completion trust --agent code-quality-improver
```

If the run produced governed-learning events, review proposals before they auto-apply:

```
> /behaviour status
> /behaviour list-proposals
```

## See also

- [Pipelines architecture](../architecture/pipelines.md)
- [Custom agents](custom-agent-workflows.md) — extending the pipeline
- [Adding an agent](../development/adding-an-agent.md) — agent definition format
- [PRD-driven development](prd-driven-development.md) — full PRD → code arc that ends in `/archon-code`
- [Research pipeline (`/archon-research`)](archon-research-pipeline.md) — sibling 46-agent pipeline for prose instead of code
- [Game-theory pipeline (`/gametheory`)](gametheory-pipeline.md) — sibling pipeline for strategic situation analysis (Tier 1 fingerprint → routing → specialists → report)
