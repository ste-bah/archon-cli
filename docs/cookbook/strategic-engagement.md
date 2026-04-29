# Strategic engagement research

Use the research-orchestrator agent to produce a comprehensive 22-document intelligence package for strategic business engagement (investor pitch, partnership, due diligence, etc.).

## Setup

The `research-orchestrator` agent is built-in. Confirm:
```
/agent info research-orchestrator
```

You should see capabilities for company-intelligence-researcher, leadership-profiler, knowledge-gap-identifier, strategic-positioning-analyst, conversation-script-writer, sales-enablement-specialist, and executive-brief-writer.

## Usage

In the TUI:
```
/run-agent research-orchestrator
```

The agent prompts for the target company and engagement type, then orchestrates the 6 specialist agents to produce:

| Document | Source agent |
|---|---|
| Company business model + market positioning | company-intelligence-researcher |
| Recent developments + technology stack | company-intelligence-researcher |
| Decision-maker profiles + influence map | leadership-profiler |
| Communication style + priorities | leadership-profiler |
| Critical knowledge gaps + targeted research | knowledge-gap-identifier |
| Value proposition customization | strategic-positioning-analyst |
| Competitive differentiation | strategic-positioning-analyst |
| Valuation analysis | strategic-positioning-analyst |
| Conversation scripts (3 angles) | conversation-script-writer |
| Discovery questions | conversation-script-writer |
| Cheat sheet | sales-enablement-specialist |
| Preparation checklist | sales-enablement-specialist |
| Follow-up playbook | sales-enablement-specialist |
| Objection handling | sales-enablement-specialist |
| Executive summary | executive-brief-writer |
| Master engagement guide | executive-brief-writer |

(The full 22-document list lives in the research-orchestrator agent definition.)

## Headless / scripted

For batch processing or CI:
```bash
archon -p "research target: Acme Corp, engagement: investor pitch" \
  --agent research-orchestrator \
  --max-budget-usd 10 \
  --output-format json > acme-research.json
```

## Output structure

Each document lands in `<workdir>/.archon/research/<target>/<document>.md`. The orchestrator also produces an index at `<target>/INDEX.md`.

## Cost expectations

Full 22-document package: ~50-80k input tokens, ~30-50k output tokens. Depending on model:
- Sonnet 4.6: ~$1-2
- Opus 4.7: ~$8-15
- Haiku 4.5: ~$0.10-0.30 (lower quality)

Set a hard limit:
```bash
archon --max-budget-usd 5.00 --agent research-orchestrator
```

## Iterating

The orchestrator accepts feedback at any phase:
```
/run-agent research-orchestrator --resume <task-id>
```

Or invoke specialist agents directly to deepen specific sections:
```
/run-agent leadership-profiler "deeper profile on Acme's CTO, focus on technical decision criteria"
```

## See also

- [Custom agents](custom-agent-workflows.md) — building your own orchestrators
- [Pipelines](../architecture/pipelines.md) — multi-agent orchestration architecture
