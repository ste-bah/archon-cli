# Running the research pipeline (`/archon-research`)

End-to-end TUI walkthrough of the 46-agent PhD research pipeline. The TUI primary is `/archon-research` — equivalent to the shell command `archon pipeline research <topic>` but driven from inside an interactive session.

> **Provider parity.** The research pipeline uses the active provider. Anthropic
> OAuth/API-key/proxy remains the default; set `[llm].provider =
> "openai-codex"` after `archon auth login --provider openai-codex` to run the
> same research workflow through Codex. The provider capability matrix is the
> source of truth if a backend limitation appears.

## When to use

The research pipeline is the right tool for:

- Literature reviews where you need cited claims, contradiction detection, and a structured synthesis chapter
- Technical deep dives that span 20+ source documents
- Policy / regulatory research that has to leave an inspectable provenance trail
- Strategic intelligence packages that combine document evidence with multi-perspective synthesis

For a quick fact-finding question, just chat normally — the 46-agent pipeline overhead isn't worth it for a single Google-able answer.

## Trigger

Inside the TUI:

```
> /archon-research impact of transformer architectures on retrieval-augmented generation since 2024
Starting research pipeline for topic: impact of transformer architectures on retrieval-augmented generation since 2024
[research-planner] decomposing topic into 4 research questions...
[research-planner] complete (3.4s, $0.06)
[literature-mapper] querying KB and document store for relevant sources...
[literature-mapper] found 47 candidate sources, ranking by quality
…
```

The handler spawns the audited pipeline async via `tokio::spawn`. Per-agent progress streams to the TUI through canonical activity events and conversation output, while prompts, attempts, accepted outputs, quality scores, run-level memory, and state are persisted under `<workdir>/.archon/pipelines/<session-id>/`. The conversation stays interactive — you can keep using other slash commands while the run is in flight.

In the TUI, each research stage is launched through the installed Archon
subagent executor, not as a bare provider prompt. That gives every agent the
same memory, document, transcript, hook, and Agent Activity plumbing used by
normal subagents, while the audited research bundle remains the source of truth
for pipeline resume and verification.

Continuation is handled by the shared pipeline control surface, not by a second
`/archon-research` invocation. If the run is interrupted, use
`/pipeline resume <session-id>` or `archon pipeline resume <session-id>`; the
persisted bundle records whether the run is coding or research.

Equivalent CLI invocation (same persisted state, same outputs):

```bash
archon pipeline research "impact of transformer architectures on retrieval-augmented generation since 2024"
```

If `--dry-run` is needed (the CLI form supports it; the slash form does not):

```bash
archon pipeline research "..." --dry-run
```

## What Happens — 8 Agent Phases, 46 Agents

The pipeline runs phases sequentially with phase-reviewer gates between each.
Each agent output is persisted to the audited bundle at
`<workdir>/.archon/pipelines/<session-id>/` for verification and inspection.
Archon also builds a research RLM pack for each later agent: a pinned chapter
structure, a phase-aware rolling window of recent accepted outputs, selected
writer outputs for validation/final assembly, and deterministic pre-scan data.
This is the host-side context handoff; research agents do not need shell/file
tools to read the bundle during the LLM call.

### Phase 1: Foundation (6 agents)

`step-back-analyzer` reframes the question against parent domains → `self-ask-decomposer` decomposes the topic into essential questions → `ambiguity-clarifier` resolves unclear terms → `research-planner` plans the research flow → `construct-definer` defines core constructs → `dissertation-architect` locks the chapter structure.

Output: framing, decomposition, definitions, research plan, constructs, and chapter structure.

### Phase 2: Discovery (4 agents)

`literature-mapper` runs a hybrid retrieval over the KB → `source-tier-classifier` ranks sources by quality → `citation-extractor` extracts citation material → `context-tier-manager` organizes hot/warm/cold context tiers.

Output: literature map, source catalog, source tiers, citation extracts, and context hierarchy.

### Phase 3: Architecture (4 agents)

`theoretical-framework-analyst` extracts theoretical lenses → `contradiction-analyzer` lists detected contradictions and resolutions → `gap-hunter` confirms what's still missing → `risk-analyst` flags conclusions sensitive to weak evidence.

Output: theoretical framework, contradiction report, research gaps, and risk assessment.

### Phase 4: Synthesis (5 agents)

`evidence-synthesizer` clusters findings → `pattern-analyst` finds recurring claims → `thematic-synthesizer` builds themes across sources → `theory-builder` constructs the theoretical model → `opportunity-identifier` identifies research and product opportunities.

Output: evidence synthesis, pattern catalog, theme hierarchy, theory development, and opportunity matrix.

### Phase 5: Design (9 agents)

`method-designer`, `hypothesis-generator`, `model-architect`, `analysis-planner`, `sampling-strategist`, `instrument-developer`, `validity-guardian`, `methodology-scanner`, and `methodology-writer` design the research method and methodology chapter.

Output: research design, hypotheses, conceptual model, analysis plan, sampling strategy, instruments, validity assessment, methodology survey, and methodology chapter.

### Phase 6: Writing (6 agents)

`introduction-writer`, `literature-review-writer`, `methodology-writer`, `results-writer`, `discussion-writer`, `conclusion-writer`. Each produces its section against the structure from Phase 1.

Output: introduction, literature review, results, discussion, conclusion, and abstract sections.

### Phase 7: Validation (11 agents)

`systematic-reviewer`, `ethics-reviewer`, `adversarial-reviewer`, `confidence-quantifier`, `citation-validator`, `reproducibility-checker`, `apa-citation-specialist`, `consistency-validator`, `quality-assessor`, `bias-detector`, and `file-length-manager` validate the draft before final assembly.

Output: systematic review, ethics review, adversarial critique, confidence scores, citation validation, reproducibility report, APA audit, consistency report, quality assessment, bias analysis, and structure audit.

### Phase 8: Final assembly (1 agent)

`chapter-synthesizer` runs after Phase 7 validation as the final agent in the
pipeline. It assembles accepted chapter outputs into the final research paper,
preserving citation and confidence context from the audited agent records.

Output: a final research paper. After `chapter-synthesizer` finishes, Archon
normalizes the accepted output into the canonical APA 7 paper structure, requires
exactly one `References` section, keeps appendices after references, and writes:

```
<workdir>/.archon/pipelines/<session-id>/exports/final-paper.md
<workdir>/.archon/pipelines/<session-id>/exports/final-paper.pdf
```

The pipeline then marks the session complete, runs completion integrity on the
final answer in the CLI path, and stores the summary in bundle state.

## Where outputs are written

The audited bundle contains both raw trace data and usable research artifacts:

| Path | Contents |
|---|---|
| `outputs/markdown/<nnn>-<agent>.md` | Canonical Markdown copy of every accepted agent output |
| `outputs/artifacts/<nnn>-<agent>/...` | Named artifacts declared by that agent, such as chapter plans or validation reports |
| `outputs/rlm/research/...` | Run-level memory namespace materialized as Markdown for inspection and resume/debugging |
| `exports/final-paper.md` | Final normalized research paper |
| `exports/final-paper.pdf` | Final rendered PDF |

The research pipeline uses ingested docs/KB/provenance and the RLM bundle for
research context. LEANN is code-context infrastructure for coding workflows and
is not initialized for `/archon-research`.

## Live progress in the TUI

The Agent Activity rail shows the parent turn plus active subagent rows live,
including provider/model/cost metadata where known:

```
─── Agent Activity ─────────────────────────────────────────────
  ▶ research-orchestrator   openai-codex/gpt-5.4  running   01:23
    └─ [AGENT] research-planner   openai-codex/gpt-5.4 done 3.4s
    └─ [AGENT] gap-hunter                          done       4.1s
    └─ [AGENT] step-back-analyzer                  done       2.8s
    └─ [AGENT] methodology-scanner                 done       3.0s
    └─ [AGENT] dissertation-architect              done       3.7s
    └─ [AGENT] literature-mapper                   running    8.2s
    └─ [AGENT] source-tier-classifier              queued     —
─────────────────────────────────────────────────────────────────
```

Rows derive from canonical activity events; each spawned subagent appears as an
`[AGENT]` row that moves `running → done | failed`.

## Status from another session

```
> /pipeline list
SESSION ID                                 KIND       PHASE       STATUS    STARTED
01HYCDF3RR…                                research   phase-3     running   2026-05-04 21:08
01HYCDC4XKM91Y…                            coding     phase-6     complete  2026-05-04 19:34

> /pipeline status 01HYCDF3RR…
> /pipeline verify 01HYCDF3RR… --write-report
> /pipeline inspect 01HYCDF3RR…
Status:    InProgress (phase 3 of 8)
Phase:     theoretical-synthesis
Last agent: thematic-synthesizer (completed 4s ago)
Cost:      $3.92 / $20.00 budget
Sources read: 47
Started:   2026-05-04 21:08:11Z
Resumeable: yes
```

## Resume after a crash

```
> /pipeline list
> /pipeline resume 01HYCDF3RR…
[recovery] verifying git working tree...
[recovery] verifying audited bundle...
[recovery] last completed agent: evidence-synthesizer
[recovery] resuming at phase-3 thematic-synthesizer
```

Resume is git-aware and verifier-gated. It refuses to continue if files under
the pipeline's purview changed unexpectedly or if persisted prompt/output
records fail hash verification.

## Inspecting after completion

```
> /pipeline status 01HYCDF3RR…
Status:    Complete
Phase:     final assembly
Total cost: $14.27
Agents run: 46 / 46
Bundle: .archon/pipelines/01HYCDF3RR…
Final paper Markdown: .archon/pipelines/01HYCDF3RR…/exports/final-paper.md
Final paper PDF: .archon/pipelines/01HYCDF3RR…/exports/final-paper.pdf
Total sources cited: 47
Final draft: 11428 words
Duration: 18m 42s
```

Inspect and export the audited trace from inside the TUI:

```
> /pipeline verify 01HYCDF3RR… --write-report
> /pipeline inspect 01HYCDF3RR…
> /pipeline export-traces 01HYCDF3RR… --out research-traces.jsonl
```

Verify the citations actually match real sources (don't trust the model's word):

```
> /completion verify 01HYCDF3RR… --agent citation-validator --model sonnet
> /completion incidents
> /completion trust --agent citation-validator
```

If the verifier flags hallucinated citations, the run is marked as a `false_completion_incident` and the citation-validator's trust score drops. Subsequent research runs auto-route around that agent if its trust is below threshold.

## Source material — pre-ingest first

The pipeline's quality is bounded by what's in your KB. Ingest source material before the run:

```
> /docs ingest ./papers/transformers-rag-2024
[ingest] 23 documents, 412 chunks, 1247 entities, 3891 claims persisted
[ingest] pack id: transformers-rag-2024

> /kb process --claims --entities --relations --contradictions
[kb] processed 412 chunks
[kb] extracted 1247 entities, 891 relations, 18 contradictions

> /archon-research "impact of transformer architectures on RAG since 2024" --kb transformers-rag-2024
```

Without an ingested KB, the pipeline falls back to model-only knowledge (whatever's in the training cutoff). With an ingested KB, every claim in the final draft is grounded in a specific persisted chunk with a provenance edge.

## Cost expectations

Full 46-agent pipeline on a moderate topic with a 50-source KB:

- ~200-400k input tokens (heavy due to KB context injection per agent)
- ~30-60k output tokens
- Sonnet 4.6: $8-20
- Opus 4.7 (heavy phases only): $20-50

Set a budget cap before starting:

```
> /archon-research "..." --budget 15
```

The pipeline halts gracefully at budget, persists checkpoints, and waits for either `/pipeline resume` (with budget extended) or `/pipeline abort`.

## Customizing

Per-project agent overrides live at:

```
<workdir>/.archon/agents/research/<agent-key>.md
```

A project-local agent definition takes precedence over the built-in. Useful when your project has domain-specific writing conventions (e.g., a particular citation style, a non-English research tradition, or industry jargon).

## See also

- [PRD-driven development](prd-driven-development.md) — `/to-prd` → `/prd-to-spec` → `/spec-to-tasks` → `/archon-code`
- [Coding pipeline (`/archon-code`)](god-code-pipeline.md) — sibling 50-agent pipeline for code instead of prose
- [Game-theory pipeline (`/gametheory`)](gametheory-pipeline.md) — sibling pipeline for strategic situation analysis (Tier 1 fingerprint → routing → specialists → report)
- [Real-world Evidence Engine](real-world-evidence-engine.md) — composing docs + KB + research + provenance + governed learning
- [Pipelines architecture](../architecture/pipelines.md)
