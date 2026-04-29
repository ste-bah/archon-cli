# Running the god-code pipeline

End-to-end walkthrough of the 50-agent coding pipeline.

## When to use

The pipeline shines on:
- New features that span multiple files / crates
- Refactors with cross-cutting concerns
- Implementations that need design + tests + review
- Tasks where you want every change reviewed by a specialized agent

For one-off edits, just chat normally — the pipeline overhead isn't worth it for trivial changes.

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

### Phase 1: Specification (5 agents)

`task-analyzer` parses your input → `requirement-extractor` pulls out functional/non-functional requirements → `requirement-prioritizer` MoSCoW-orders them → `scope-definer` sets boundaries → `feasibility-analyzer` validates technical feasibility.

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

The pipeline reads its agent definitions from `crates/archon-pipeline/src/agents/coding/*.md` plus TOML manifests. Override per-project:

```
<workdir>/.archon/agents/coding/code-quality-improver.md
```

A project-local agent definition takes precedence over the built-in.

## Dev flow gates (separate concept)

Don't confuse the pipeline's 5 deterministic gates with the `dev-flow` gates run by `scripts/dev-flow-gate.sh`. The latter is a project-internal CI/build flow:

| Gate | Check |
|---|---|
| 1. tests-written-first | Test file exists BEFORE implementation |
| 2. implementation-complete | Code compiles, no errors |
| 3. sherlock-code-review | Sherlock adversarial review of implementation |
| 4. tests-passing | All tests pass (include count) |
| 5. live-smoke-test | Feature actually invoked end-to-end |
| 6. sherlock-final-review | Sherlock final review: integration + wiring verified |

See [Dev flow gates](../development/dev-flow-gates.md) for the project-internal protocol.

## See also

- [Pipelines architecture](../architecture/pipelines.md)
- [Custom agents](custom-agent-workflows.md) — extending the pipeline
- [Adding an agent](../development/adding-an-agent.md) — agent definition format
