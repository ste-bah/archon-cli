# Running the god-code pipeline

End-to-end walkthrough of the 48-agent coding pipeline.

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

```
/archon-code "implement OAuth2 token refresh with file locking"
```

Or via CLI:
```bash
archon pipeline code "implement OAuth2 token refresh with file locking"

# Dry run (plan without executing)
archon pipeline code "..." --dry-run
```

## What happens

The pipeline runs 6 phases sequentially. Each phase has reviewers that gate progression to the next.

### Phase 1: Understanding (8 agents)

`contract-agent` parses the input contract → `requirement-extractor` pulls out functional/non-functional requirements → `requirement-prioritizer` MoSCoW-orders them → `scope-definer` sets boundaries → `context-gatherer` reads existing code → `feasibility-analyzer` validates technical feasibility → `pattern-explorer` identifies relevant patterns → `technology-scout` evaluates external solutions.

Output: `specification.json` with structured requirements, scope, feasibility verdict.

### Phase 2: Exploration (5 agents)

`context-gatherer` reads existing code → `codebase-analyzer` maps architecture → `pattern-explorer` identifies relevant patterns → `technology-scout` evaluates external solutions → `ambiguity-clarifier` resolves unknowns.

Output: `exploration.json` with codebase map, patterns to follow, unknowns flagged.

### Phase 3: Architecture (7 agents)

`system-designer` does high-level → `component-designer` does internal → `interface-designer` defines APIs → `data-architect` designs storage → `security-architect` flags threats → `integration-architect` plans external connections → `performance-architect` plans for load.

Output: `architecture.json` with full design.

### Phase 4: Implementation (12 agents)

Splits the work: `code-generator`, `unit-implementer`, `service-implementer`, `api-implementer`, `frontend-implementer`, `data-layer-implementer`, `type-implementer`, `error-handler-implementer`, `logger-implementer`, `config-implementer`, `integration-tester`, `dependency-manager`.

Each writes its slice in parallel where possible. Output: actual code in `<workdir>` plus `implementation/` artefacts (types, tests, error specs).

### Phase 5: Quality (7 agents)

`code-quality-improver`, `sherlock-holmes` (forensic review), `security-tester`, `regression-tester`, `coverage-analyzer`, `code-simplifier`, `final-refactorer`. The Sherlock Holmes agent independently re-reads the code; reviews from other agents are not trusted.

Output: `quality.json` with findings, test results, refactor suggestions.

### Phase 6: Sign-off (8 agents)

`sign-off-approver` plus phase-1 through phase-6 reviewers. Each phase is checked once more. Final approval gates the pipeline closing.

Output: `signoff.json`. Pipeline marks the session as complete.

## Monitoring progress

```
# In another terminal
archon pipeline status <session-id>
archon pipeline list
```

The TUI shows live progress with phase indicators.

## Resuming

If archon-cli crashes or you `Ctrl-C`:
```bash
archon pipeline list                      # find your session
archon pipeline resume <session-id>       # continues from last completed gate
```

Resume requires git working tree consistency — if files changed mid-pipeline, the recovery layer rejects continuation.

## Aborting

```bash
archon pipeline abort <session-id>
```

Cleans up partial state, preserves the ledger for forensic review.

## Cost expectations

Full 48-agent pipeline on a moderate task (e.g., new feature spanning 3 crates):
- ~150-300k input tokens (heavy due to L0-L3 layered context)
- ~20-50k output tokens
- Sonnet 4.6: $5-15
- Opus 4.7 (heavy phases only): $15-40

Set a hard limit:
```bash
archon pipeline code "..." --max-budget-usd 20
```

## Customizing

The pipeline reads its agent definitions from `crates/archon-pipeline/src/agents/coding/*.md` plus TOML manifests. Override per-project:

```
<workdir>/.archon/agents/coding/code-quality-improver.md
```

A project-local agent definition takes precedence over the built-in.

## Dev flow gates (separate concept)

Don't confuse the pipeline's deterministic gates (between phases) with archon-cli's CI gates (`scripts/ci-gate.sh`). The pipeline gates govern phase transitions during a `/archon-code` run; the CI gates govern code quality before merge. Different concerns.

See [CI gates](../development/dev-flow-gates.md) for the technical CI flow (file-size, banned-imports, fmt, clippy, test, baseline diff, bench compile-check).

## End-to-end TUI walkthrough

What driving a coding-pipeline run from inside the TUI actually looks like. Assumes you're at the `archon` prompt and authenticated.

### Discover-and-plan loop (recommended)

Always dry-run first. The plan is cheap (no LLM cost) and tells you whether the pipeline understood your task before you spend $5-15 on a real run.

```
> archon pipeline code "Add archon docs summarize <doc-id>: read persisted chunks, produce cited summaries, write provenance edges, add tests, update docs" --dry-run
```

What you see:

```
=== Coding Pipeline Dry Run ===
Task: Add archon docs summarize <doc-id>: read persisted chunks, produce cited
      summaries, write provenance edges, add tests, update docs

Agent Sequence (48 agents):
  Phase 1: task-analyzer, requirement-extractor, requirement-prioritizer
  Phase 2: pattern-explorer, technology-scout, feasibility-analyzer, codebase-analyzer
  Phase 3: system-designer, component-designer, interface-designer, ...
  Phase 4: code-generator, unit-implementer, api-implementer, ...
  Phase 5: test-generator, integration-tester, security-tester, ...
  Phase 6: final-refactorer, sign-off-approver

Estimated cost: ~$2.50-5.00 (varies by task complexity)
```

Re-run without `--dry-run` to actually execute. Pin a budget cap:

```
> archon pipeline code "Add archon docs summarize..." --max-budget-usd 20
```

### Live progress in the TUI

Once the pipeline is running, the TUI Agent Activity rail (added in v0.1.40) shows the parent turn plus active subagent tool calls live:

```
─── Agent Activity ─────────────────────────────────────────────
  ▶ pipeline-coordinator                         running   00:42
    └─ [AGENT] task-analyzer                     done       3.1s
    └─ [AGENT] requirement-extractor             done       4.8s
    └─ [AGENT] requirement-prioritizer           running    1.2s
─────────────────────────────────────────────────────────────────
```

The rail derives rows from existing `ToolStart` / `ToolComplete` events, so every spawned subagent appears as a `[AGENT]` row that moves `running → done | failed`.

### Status from another TUI session (or the same one)

Don't lose your run. Open a second `archon` session in another terminal, or use the slash form in the same one:

```
> /pipeline list
SESSION ID                                 PHASE       STATUS    STARTED
01HYCD3WSXKJ8R…                            phase-3     running   2026-05-04 19:12
01HYCD0GMQ1YZP…                            phase-6     complete  2026-05-04 18:01

> /pipeline status 01HYCD3WSXKJ8R…
Status:    InProgress (phase 3 of 6)
Phase:     architecture
Last agent: component-designer (completed 8s ago)
Cost:      $4.31 / $20.00 budget
Started:   2026-05-04 19:12:33Z
Resumeable: yes
```

### Resume after a crash

If archon dies mid-pipeline (Ctrl-C, OOM, network blip), restart it and pick up where you left off:

```
$ archon
> /pipeline list
> /pipeline resume 01HYCD3WSXKJ8R…
[recovery] verifying git working tree...
[recovery] last completed gate: phase-2 sign-off-approver
[recovery] resuming at phase-3 system-designer
```

The recovery layer refuses to resume if files under the pipeline's purview have changed since the last gate — protects against silently overwriting user edits. Commit or stash first if that fires.

### Abort cleanly

```
> /pipeline abort 01HYCD3WSXKJ8R…
[abort] killing in-flight subagents...
[abort] partial state cleaned, ledger preserved at .archon/pipelines/01HYCD3WSXKJ8R…/ledger.jsonl
```

Forensic-review-friendly: the ledger stays so you can reconstruct what each agent did.

### Inspecting after completion

```
> /pipeline status 01HYCD0GMQ1YZP…
Status:    Complete
Phase:     phase-6 sign-off
Total cost: $11.48
Agents run: 48 / 48
Files modified: crates/archon-docs/src/summarize.rs (new), tests/docs_summarize_smoke.rs (new), 4 others
```

Then verify the claims rather than trust the final paragraph:

```
> archon completion verify 01HYCD0GMQ1YZP… --agent code-quality-improver --model sonnet
> archon completion incidents
> archon completion trust --agent code-quality-improver
```

If the run produced governed-learning events, review proposals before they auto-apply:

```
> archon behaviour status
> archon behaviour list-proposals
```

## See also

- [Pipelines architecture](../architecture/pipelines.md)
- [Custom agents](custom-agent-workflows.md) — extending the pipeline
- [Adding an agent](../development/adding-an-agent.md) — agent definition format
- [PRD-driven development](prd-driven-development.md) — full PRD → code arc that ends in `/archon-code`
