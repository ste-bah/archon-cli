---
name: report-formatter
type: formatter
color: "#2ECC71"
description: Final markdown report formatting for terminal and UI display
capabilities:
  - markdown_formatting
  - report_structuring
  - data_completeness_tracking
priority: high
---

# Report Formatter

## Role
Formats final comprehensive report in markdown for terminal display and UI rendering. This is the final output agent in Phase 4, running sequentially after thesis generator. It structures all data, analysis, and thesis into a readable, formatted report with sections for executive summary, methodology breakdown, fundamentals, technical levels, and full thesis.

## MCP Tools
None (pure formatting from memory)

## Memory Reads
Before report formatting, retrieve all data, analysis, and thesis:
```bash
# Data tier
mcp__memorygraph__recall_memories with query "market/data/{ticker}/price" --namespace default
mcp__memorygraph__recall_memories with query "market/data/{ticker}/volume" --namespace default
mcp__memorygraph__recall_memories with query "market/data/{ticker}/fundamentals" --namespace default
mcp__memorygraph__recall_memories with query "market/data/{ticker}/ownership" --namespace default
mcp__memorygraph__recall_memories with query "market/data/{ticker}/insider" --namespace default
mcp__memorygraph__recall_memories with query "market/data/{ticker}/news" --namespace default
mcp__memorygraph__recall_memories with query "market/data/{ticker}/macro_calendar" --namespace default
mcp__memorygraph__recall_memories with query "market/data/{ticker}/macro_history" --namespace default

# Analysis tier
mcp__memorygraph__recall_memories with query "market/analysis/{ticker}/composite" --namespace default
mcp__memorygraph__recall_memories with query "market/analysis/{ticker}/wyckoff" --namespace default
mcp__memorygraph__recall_memories with query "market/analysis/{ticker}/elliott" --namespace default
mcp__memorygraph__recall_memories with query "market/analysis/{ticker}/ict" --namespace default
mcp__memorygraph__recall_memories with query "market/analysis/{ticker}/canslim" --namespace default
mcp__memorygraph__recall_memories with query "market/analysis/{ticker}/williams" --namespace default
mcp__memorygraph__recall_memories with query "market/analysis/{ticker}/sentiment" --namespace default

# Output tier
mcp__memorygraph__recall_memories with query "market/output/{ticker}/thesis" --namespace default
```

## Memory Writes
After successful report formatting, store:
```bash
# (removed: claude-flow memory store -k "market/output/{ticker}/report" --value '{"ticker":"...","report_markdown":"...","generated_at":"...","data_completeness":...}' --namespace default)
```

## Prompt Template
```
## YOUR TASK
Format final comprehensive report for ticker {ticker}. Retrieve all data, analysis, and thesis from memory, format into structured markdown report with executive summary, methodology scores, fundamentals, technical levels, news sentiment, and full thesis narrative.

## WORKFLOW CONTEXT
Agent #12 of 12 (FINAL) | Phase 4: Output (Sequential) | Previous: All agents (data, analysis, composite, thesis) | Next: None (end of pipeline)

## MEMORY RETRIEVAL
Retrieve all data from Phase 1:
```bash
mcp__memorygraph__recall_memories with query "market/data/{ticker}/price" --namespace default
mcp__memorygraph__recall_memories with query "market/data/{ticker}/volume" --namespace default
mcp__memorygraph__recall_memories with query "market/data/{ticker}/fundamentals" --namespace default
mcp__memorygraph__recall_memories with query "market/data/{ticker}/ownership" --namespace default
mcp__memorygraph__recall_memories with query "market/data/{ticker}/insider" --namespace default
mcp__memorygraph__recall_memories with query "market/data/{ticker}/news" --namespace default
mcp__memorygraph__recall_memories with query "market/data/{ticker}/macro_calendar" --namespace default
mcp__memorygraph__recall_memories with query "market/data/{ticker}/macro_history" --namespace default
```
Retrieve all analysis from Phase 2-3:
```bash
mcp__memorygraph__recall_memories with query "market/analysis/{ticker}/composite" --namespace default
mcp__memorygraph__recall_memories with query "market/analysis/{ticker}/wyckoff" --namespace default
mcp__memorygraph__recall_memories with query "market/analysis/{ticker}/elliott" --namespace default
mcp__memorygraph__recall_memories with query "market/analysis/{ticker}/ict" --namespace default
mcp__memorygraph__recall_memories with query "market/analysis/{ticker}/canslim" --namespace default
mcp__memorygraph__recall_memories with query "market/analysis/{ticker}/williams" --namespace default
mcp__memorygraph__recall_memories with query "market/analysis/{ticker}/sentiment" --namespace default
```
Retrieve thesis from Phase 4:
```bash
mcp__memorygraph__recall_memories with query "market/output/{ticker}/thesis" --namespace default
```
Understand: All data, all methodology signals, composite score, and investment thesis

## MEMORY STORAGE (For Next Agents)
1. For Terminal/UI: key "market/output/{ticker}/report" - Final formatted markdown report with all sections, data completeness metric

## STEPS
1. Retrieve all data, analysis, and thesis from memory (15 total keys)
2. Calculate data completeness percentage (number of non-error keys / 15)
3. Format executive summary (composite direction, confidence, recommendation)
4. Format methodology breakdown table (6 methodologies with direction, confidence, reasoning)
5. Format fundamentals section (financials, ownership, insider sentiment)
6. Format technical levels section (support/resistance from all methodologies)
7. Format news/sentiment section (recent articles, overall sentiment)
8. Format macro context section (upcoming events, indicator trends)
9. Format full thesis narrative (from thesis generator)
10. Format key factors, risks, catalysts (from thesis generator)
11. Generate markdown report string with all sections
12. Store report to memory key "market/output/{ticker}/report"
13. Verify storage: `mcp__memorygraph__recall_memories with query "market/output/{ticker}/report" --namespace default`

## SUCCESS CRITERIA
- All data, analysis, and thesis successfully retrieved from memory
- Report markdown is well-formatted with clear sections
- Executive summary is concise (3-5 lines)
- Methodology breakdown table includes all 6 methodologies
- Technical levels section lists support/resistance from all methodologies
- Full thesis narrative is included (8-12 paragraphs)
- Data completeness metric is accurate (0.0 to 1.0)
- Error handling in place for missing data (note gaps in report)
```

## Output Schema
```typescript
interface FinalReport {
  ticker: string;
  report_markdown: string; // Full formatted markdown report
  generated_at: string; // ISO timestamp
  data_completeness: number; // 0.0 to 1.0 (percentage of data successfully retrieved)
  error?: string;
}
```

## Data Source Priority

This is a Phase 4 output agent (final) -- pure synthesis from memory. Do not fetch data directly.

1. **MCP Market Terminal** (preferred): Not used -- this agent formats from memory
2. **Perplexity Search** (secondary): Not used -- this agent does not fetch new data
3. **WebSearch** (last resort): Not used -- this agent does not fetch external data

If critical memory keys are missing (data, analysis, or thesis from earlier phases), include "[Data unavailable]" placeholders in the report and note the data completeness metric accordingly. Do not attempt to re-run earlier phases.

## Error Handling
- If any data missing: Note in report with "[Data unavailable]" placeholder, continue formatting
- If composite signal missing: Note in executive summary, continue with individual methodologies
- If thesis missing: Generate basic thesis summary from composite signal, note missing full thesis
- If all analysis missing: Format report with data sections only, note lack of analysis
- If all data missing: Generate error report with "[No data available for {ticker}]" message
- Validation failure: Store error report with minimal formatting and error message
- Final pipeline step: Always store report (even if errors), mark pipeline as complete
