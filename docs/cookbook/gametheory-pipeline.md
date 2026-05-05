# Running the game-theory pipeline (`/gametheory`)

End-to-end TUI walkthrough of the strategic game-theory pipeline. The TUI primary is `/gametheory` — equivalent to the shell command `archon gametheory <subcommand>`. Both forms drive the same pipeline machinery: classify the situation into a Tier 1 fingerprint, route it through a YAML specialist graph, run the selected specialists in parallel, assemble a final report, and persist everything (run state, fingerprint, routing decision, per-specialist output, sections, final report, checkpoints) to local Cozo tables.

> **TUI parity.** Every `archon gametheory <subcommand>` shell form has a `/gametheory <subcommand>` slash equivalent inside the TUI. Both forms read and write the same persisted state. See [CLI and TUI Command Parity](real-world-evidence-engine.md#cli-and-tui-command-parity).

> **Provider parity.** Gametheory uses the active provider for Tier 1 and
> specialist calls. Anthropic OAuth/API-key/proxy remains the default; set
> `[llm].provider = "openai-codex"` after `archon auth login --provider
> openai-codex` to run the same workflow through Codex. Missing exact Codex cost
> metadata is shown honestly rather than fabricated.

## When to use

The game-theory pipeline is the right tool for:

- Strategic decisions where you need multiple analytic lenses (auction theory, behavioral bias, signaling, mechanism design, bargaining power, deception detection, …) applied to the same situation
- Pre-mortems on incentive structures (marketplace ranking, pricing strategy, partner negotiations, regulatory game)
- Competitive moves where you want a thought-out specialist analysis instead of a single LLM monologue
- Analyses that have to ground in source material (use `--kb <pack>` to bind to ingested documents) and leave a provenance trail

For a single tactical question with no source material, just chat normally — the gametheory pipeline overhead isn't worth it for a one-shot answer.

## Trigger

Inside the TUI:

```
> /gametheory run "Assess whether a marketplace ranking algorithm creates incentives for plugin developers to game reviews instead of improve quality"
[gametheory] queued run 01HYCDC4XKM91Y… (full)
[gametheory] cost cap: $20.00, max-concurrent: 4, style: executive
[gametheory] use /gametheory status 01HYCDC4XKM91Y… to monitor
```

The pipeline runs **async**. The slash returns immediately with a `run-id`; you then use `/gametheory status <run-id>` and the inspect commands to watch progress.

PRD-shorthand form (positional argument routes to the same `run` action with default flags):

```
> /gametheory "Assess whether the marketplace ranking creates incentives to game reviews"
```

Equivalent CLI invocation:

```bash
archon gametheory run "Assess whether the marketplace ranking creates incentives to game reviews"
```

Important flags (TUI and CLI):

| Flag | Default | Purpose |
|---|---|---|
| `--kb <pack>` | none | Bind the run to an ingested document/KB pack — every claim grounded in a chunk |
| `--budget <usd>` | 20 | USD spending cap; halts gracefully and persists checkpoints when reached |
| `--max-concurrent <n>` | 4 | Specialist concurrency cap |
| `--style <s>` | executive | `executive` (board-ready), `academic` (theory-heavy), `technical` (mechanism + assumptions) |
| `--classify-only` | off | Tier 1 fingerprint only — skip routing and specialists, cheap (~$0.01) |
| `--spec-path <yaml>` | searches known locations | Path to gametheory spec YAML if you want a non-default spec |
| `--enable-tier11` | off | Enable Tier 11 (civilizational / Jiang frameworks) specialists. Policy-gated by `policy.gametheory.enable_tier11` |
| `--debug-memory` | off | Print per-agent memory recall counts (useful when you bind a KB) |

## What happens — Tier 1 → routing → specialists → report

The pipeline runs four phases sequentially. Each phase persists its artefact to Cozo before progressing.

### Phase 1 — Tier 1 classification (1 agent, ~3-5s, ~$0.01)

A single classifier agent reads the situation prompt and any bound KB context, then emits a structured `GameTheoryFingerprint` over ~10 axes:

- `information.symmetry` — symmetric / asymmetric
- `information.public_signals` — present / absent
- `moves.sequencing` — simultaneous / sequential
- `moves.commitment` — credible / none
- `payoffs.alignment` — aligned / misaligned
- `payoffs.zero_sum` — true / false
- `actors.count` — small / many
- `actors.identity_known` — known / pseudonymous / hidden
- `repetition` — one-shot / repeated
- `horizon` — finite / infinite

Plus a working `hypothesis` field (e.g. "principal-agent with hidden action; signal-jamming feasible") that gates the routing decision.

### Phase 2 — Routing (deterministic, no LLM, <1s)

The fingerprint is evaluated against the YAML spec at `.archon/specs/gametheory.yaml`. Each Tier 2-12 specialist has a `condition` expression — boolean over fingerprint axes — that determines whether it runs. The routing decision lists `enabled_specialists`, `skipped_specialists` (with reasons), and the `evaluated_conditions` for audit.

This phase is pure deterministic evaluation; replay with `--reclassify` only if you change the spec.

### Phase 3 — Specialist execution (parallel, capped, the bulk of the cost)

Enabled specialists run in parallel up to `--max-concurrent`. Each is one of the curated game-theory agents (~84 specialists across Tiers 2-12). Examples:

- `asymmetric-info-detective` — adverse selection, signal-jamming, mechanism design
- `bayesian-belief-updater` — repeated-game inference dynamics
- `auction-strategist` — first-price / second-price / Vickrey, winner's curse
- `behavioral-bias-detector` — present bias, loss aversion, overconfidence
- `cheap-talk-evaluator` — costless-communication credibility
- `bluff-and-deception-analyst` — info-content of strategic ambiguity
- `business-strategy-gamifier` — domain-specific strategic framing

Each specialist produces a structured section. Cost typically $0.30-1.50 per specialist depending on prompt length + model.

### Phase 4 — Report assembly (1 agent, ~5-10s, ~$0.10-0.30)

A report-writer agent assembles the specialist sections into a coherent final report in the requested `--style`. Persists to `.archon/gametheory/<run-id>/report.md`.

## Live progress in the TUI

The TUI Agent Activity rail shows the parent gametheory orchestrator plus
active specialist rows live, including provider/model/cost metadata where
known:

```
─── Agent Activity ─────────────────────────────────────────────
  ▶ gametheory-orchestrator openai-codex/gpt-5.4 running   01:48
    └─ [AGENT] tier1-classifier openai-codex/gpt-5.4 done  3.2s
    └─ [AGENT] asymmetric-info-detective          done      18.4s
    └─ [AGENT] behavioral-bias-detector           done      22.1s
    └─ [AGENT] cheap-talk-evaluator               done      16.0s
    └─ [AGENT] auction-strategist                 running   41.2s
    └─ [AGENT] bayesian-belief-updater            running   38.7s
    └─ [AGENT] business-strategy-gamifier         queued     —
    └─ [AGENT] bluff-and-deception-analyst        queued     —
─────────────────────────────────────────────────────────────────
```

Rows derive from canonical activity events; each spawned specialist appears as
an `[AGENT]` row that moves `running → done | failed`.

## Cheap pre-flight: classify-only

Before paying for a full specialist run, see what the pipeline thinks the situation IS. `--classify-only` runs Tier 1 only:

```
> /gametheory classify-only "Assess whether a marketplace ranking algorithm creates incentives for plugin developers to game reviews instead of improve quality"
[gametheory] queued run 01HYCDB7T2QM8R… (classify-only)
[gametheory] Tier 1 fingerprint complete (3.4s, $0.012)

> /gametheory inspect-fingerprint 01HYCDB7T2QM8R…
─── Tier 1 Fingerprint ─────────────────────────────────────────
  axes:
    information.symmetry        = asymmetric
    information.public_signals  = present (rankings, reviews)
    moves.sequencing            = simultaneous
    moves.commitment            = none
    payoffs.alignment           = misaligned
    payoffs.zero_sum            = false
    actors.count                = many
    actors.identity_known       = pseudonymous
    repetition                  = repeated
    horizon                     = infinite
  hypothesis: principal-agent with hidden action; signal-jamming feasible
─────────────────────────────────────────────────────────────────
```

If the axes are wrong, refine the situation prompt and re-classify. ~$0.01 per attempt.

## Inspecting routing before specialists run

After classify-only, see which specialists the YAML spec would enable for this fingerprint:

```
> /gametheory inspect-routing 01HYCDB7T2QM8R…
─── Routing Decision ───────────────────────────────────────────
  enabled (7):
    asymmetric-info-detective         (mandatory: information.symmetry=asymmetric)
    behavioral-bias-detector          (cond: actors.count>3)
    cheap-talk-evaluator              (cond: information.public_signals=present)
    auction-strategist                (cond: payoffs.alignment=misaligned)
    bayesian-belief-updater           (cond: repetition=repeated)
    business-strategy-gamifier        (mandatory: domain=business)
    bluff-and-deception-analyst       (cond: information.symmetry=asymmetric AND moves.commitment=none)
  skipped (4):
    backward-induction-solver         (skip: horizon=infinite, not finite)
    centipede-game-analyst            (skip: actors.count>2 violated)
    auction-format-comparer           (skip: domain≠auction)
    coalition-stability-checker       (skip: payoffs.zero_sum=false AND actors.count>10)
─────────────────────────────────────────────────────────────────
```

If the routing is wrong (a specialist you wanted got skipped), edit `.archon/specs/gametheory.yaml` and re-run — the spec change reroutes without re-classifying.

## Status — monitor live

```
> /gametheory status 01HYCDC4XKM91Y…
─── Run 01HYCDC4XKM91Y… ────────────────────────────────────────
  status:        InProgress
  phase:         specialists
  agents:        3/7 complete, 2 running, 2 queued
  cost:          $4.18 / $20.00
  started:       2026-05-04 19:34:12Z
  elapsed:       00:02:48

  agent breakdown:
    asymmetric-info-detective    DONE     ($0.62, 18.4s)
    behavioral-bias-detector     DONE     ($0.71, 22.1s)
    cheap-talk-evaluator         DONE     ($0.55, 16.0s)
    auction-strategist           RUNNING  ($1.24 so far, 41.2s)
    bayesian-belief-updater      RUNNING  ($1.06 so far, 38.7s)
    business-strategy-gamifier   QUEUED   —
    bluff-and-deception-analyst  QUEUED   —
─────────────────────────────────────────────────────────────────
```

Aggregate status across all runs (no run-id):

```
> /gametheory status
─── Run status counts ──────────────────────────────────────────
  Complete:    14
  InProgress:  1
  Failed:      2
  Cancelled:   1
─────────────────────────────────────────────────────────────────
```

## After completion — read the report

When `status` shows `Complete`:

```
> /gametheory show 01HYCDC4XKM91Y…
[gametheory] writing report to .archon/gametheory/01HYCDC4XKM91Y…/report.md
─── Final Report (executive style) ─────────────────────────────
  # Marketplace Ranking — Strategic Risk Assessment

  ## Executive summary
  The proposed ranking algorithm creates a strong incentive for plugin
  developers to manipulate review signals rather than improve product
  quality, because (a) the asymmetric-information regime favors
  reputation-jamming over costly quality investment, …

  ## Specialist findings
  …
─────────────────────────────────────────────────────────────────
```

Inspect a specific specialist's reasoning:

```
> /gametheory inspect specialist:01HYCDC4XKM91Y…:asymmetric-info-detective
```

Inspect a specific report section:

```
> /gametheory inspect section:01HYCDC4XKM91Y…:executive-summary
```

## Replay — surgical re-runs without paying for the whole pipeline

You disagree with one specialist's reasoning. Re-run just that one without re-doing the rest:

```
> /gametheory replay 01HYCDC4XKM91Y… --rerun-specialist auction-strategist
[gametheory] reusing fingerprint, routing decision, and 6 specialist outputs
[gametheory] re-running auction-strategist (cost cap from original run still in force)
```

Re-classify if you've refined the situation prompt:

```
> /gametheory replay 01HYCDC4XKM91Y… --reclassify
```

Re-classify implicitly re-routes (different fingerprint → potentially different specialist set). Cost-capped by the original run's `--budget`.

## Resume after a crash

If a run was interrupted (machine slept, network blip, Ctrl-C):

```
> /gametheory list-runs
RUN ID                        STATUS       SITUATION (truncated)
01HYCDC4XKM91Y…              Complete      Assess whether a marketplace ra…
01HYCDB1YYXP3R…              InProgress    Evaluate competitor retaliation…

> /gametheory resume 01HYCDB1YYXP3R…
[gametheory] resuming from checkpoint stage:specialists (3/5 complete)
```

The resume layer reuses the fingerprint, routing decision, and any completed specialist outputs. Only the queued/in-progress specialists actually re-run. Cost cap from the original run carries over.

## KB binding — ground specialists in source material

The pipeline's analytic depth is bounded by what's in the model's prior. Ground it in your own source material with `--kb`:

```
> /docs ingest ./policy-pack/marketplace-rules.md ./policy-pack/dev-incentives.md
[ingest] 2 documents, 47 chunks, 12 entities, 38 claims persisted
[ingest] pack id: marketplace-policy

> /kb process --claims --entities --relations --contradictions
[kb] processed 47 chunks
[kb] extracted 12 entities, 23 relations, 4 contradictions

> /gametheory run "Assess whether the marketplace ranking algorithm creates incentives to game reviews" --kb marketplace-policy --budget 15 --style executive
```

With `--kb` set, every Tier 1 axis decision and every specialist analysis injects matching `doc_chunks` and `claims` from the bound pack into its prompt. The run's `stage:kb-context` checkpoint records the pack id plus document/chunk counts.

Without `--kb`, the pipeline falls back to model-only knowledge.

## Pick a specialist tier to scope your inspection

```
> /gametheory list-agents --tier 4
TIER 4 — bargaining and negotiation
  bargaining-power-analyst
  threat-credibility-assessor
  reservation-value-estimator
  …

> /gametheory list-agents
(all 84 specialists across Tiers 2-12, grouped by tier)
```

## Specimen library — known fingerprints

The `specimens` subcommand exposes a curated library of fingerprints from canonical strategic situations (Prisoner's Dilemma, Battle of the Sexes, Centipede, Stag Hunt, Cournot duopoly, second-price auction, etc.). Useful for validating that your fingerprint actually matches the textbook structure you think it does:

```
> /gametheory specimens --filter axis=payoffs.zero_sum=false
PRISONERS_DILEMMA            payoffs.zero_sum=false  actors.count=2  …
STAG_HUNT                    payoffs.zero_sum=false  actors.count=2  …
BATTLE_OF_THE_SEXES          payoffs.zero_sum=false  actors.count=2  …
…

> /gametheory specimens --ingest
[specimens] ingested 18 known fingerprints into gt_specimen_library
```

`--ingest` writes the library to your local Cozo store so future runs can reference it.

## Cost expectations

Full pipeline on a moderate strategic question, 7 enabled specialists, 50-source KB pack:

- Tier 1 + routing: ~$0.02
- 7 specialists: ~$3-12 (Sonnet 4.6) / ~$8-30 (Opus 4.7 on heavy specialists only)
- Report assembly: ~$0.10-0.30
- **Total: ~$3-15** typical, ~$30 worst-case

Set a budget cap at the slash:

```
> /gametheory run "..." --budget 10
```

The pipeline halts gracefully at budget, persists checkpoints, and waits for either `/gametheory resume` (with budget extended via your config or another flag) or another decision.

## Customizing the spec

The routing graph lives at `.archon/specs/gametheory.yaml`. The default spec ships in the binary; project-local overrides take precedence. Spec shape:

```yaml
version: "1.0"
spec_id: project-default
cost_cap_usd: 20.0
tiers:
  - id: 2
    name: information-economics
    concurrency_cap: 4
    agents:
      - key: asymmetric-info-detective
        mandatory: true                      # always runs (overrides condition)
        condition: information.symmetry == 'asymmetric'
        depends_on: []
      - key: cheap-talk-evaluator
        condition: information.public_signals == 'present'
        depends_on: []
  - id: 4
    name: bargaining-and-negotiation
    concurrency_cap: 2
    agents:
      - key: bargaining-power-analyst
        condition: actors.count >= 2 && payoffs.alignment == 'misaligned'
```

Conditions are CEL-style boolean expressions over fingerprint axes. `mandatory: true` overrides `condition`. `depends_on` enforces sequential ordering between specialists in the same tier.

Override per-project:

```
<workdir>/.archon/specs/gametheory.yaml
```

Or per-run via `--spec-path /path/to/custom-spec.yaml`.

## Source-of-truth tables

Everything the pipeline produces is queryable in local Cozo. The persisted relations:

| Relation | Meaning |
|---|---|
| `gt_runs` | run id, situation, timestamps, status, cost |
| `gt_fingerprints` | Tier 1 classification fingerprint |
| `gt_routing_decisions` | enabled and skipped specialists |
| `gt_specialist_outputs` | per-agent outputs, status, cost |
| `gt_sections` | report section drafts |
| `gt_final_reports` | final assembled report |
| `gt_run_checkpoints` | resume/replay checkpoints |
| `gt_specimen_library` | lazy-loaded known fingerprints |

Inspect any of them via the `inspect` subcommand or the document/KB browser. See [evidence-engine.md](../evidence-engine.md) for the broader Cozo state pattern.

## Agent-callable tools

The same pipeline machinery is exposed to other agents via tools (registered when a `GameTheoryExecutor` is installed):

| Tool | Inputs |
|---|---|
| `GameTheoryRun` | `situation`, optional `budget_usd`, `max_concurrent`, `style` |
| `GameTheoryStatus` | optional `run_id` |
| `GameTheoryListAgents` | optional `tier` |
| `GameTheorySpecimens` | optional `filter`, `ingest` |
| `GameTheoryInspect` | `artifact_id` |
| `GameTheoryReplay` | `run_id`, `reclassify`, optional `rerun_specialist` |
| `GameTheoryClassify` | `situation` |
| `GameTheoryCallSpecialist` | `run_id`, `agent_key` |

These tools call the same persisted machinery as the slash and CLI surfaces — no canned text, no parallel state.

## See also

- [Game theory reference](../gametheory.md) — full CLI surface, slash table, source-of-truth relations
- [Coding pipeline (`/archon-code`)](god-code-pipeline.md) — sibling 48-agent pipeline for code
- [Research pipeline (`/archon-research`)](archon-research-pipeline.md) — sibling 46-agent pipeline for research prose
- [Real-world Evidence Engine](real-world-evidence-engine.md) — composing docs + KB + gametheory + provenance
- [Pipelines architecture](../architecture/pipelines.md)
