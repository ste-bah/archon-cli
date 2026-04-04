---
name: list-agents
description: List all custom agents with metadata (creation date, last used, invocation count). Supports --verbose and --all flags.
triggers:
  - /list-agents
  - list agents
  - show agents
---

# /list-agents -- Show Available Custom Agents

## Step 1: Scan Agent Directories

Read all subdirectories in `.claude/agents/custom/` using Glob or Bash `ls`.
Exclude `_template` and `_behavior_schema` directories.

## Step 2: Load Metadata

For each agent directory:
1. Read `meta.json` if it exists, parse as JSON
2. Read first 3 lines of `agent.md` for role summary (--verbose only)
3. Count files in the directory
4. If `meta.json` missing: use filesystem timestamps as fallback
5. If `agent.md` missing: mark as `[INVALID]`

## Step 3: Check for Archived (--all flag only)

If `--all` flag present, also scan `.claude/agents/archived/` and include with `[ARCHIVED]` tag.

## Step 4: Display

### Standard Output

```
## Custom Agents ({count} total)

| Name | Created | Last Used | Invocations | Files |
|------|---------|-----------|-------------|-------|
| sec-filing-analyzer | 2026-03-30 | 2026-03-30 | 5 | 6 |
| code-reviewer | 2026-03-30 | 2026-03-30 | 3 | 6 |
| doc-writer | 2026-03-30 | — | 0 | 6 |

Run `/run-agent {name} "task"` to invoke an agent.
Run `/create-agent "description"` to create a new one.
```

### Verbose Output (--verbose)

Same table plus:
```
### sec-filing-analyzer
> Analyze SEC filings (10-K, 10-Q, 8-K) to identify revenue recognition risks...
Quality: effective_rate=0.80, invocations=5

### code-reviewer
> Perform meticulous code review of Python and TypeScript codebases...
Quality: effective_rate=0.67, invocations=3
```

### Edge Cases
- No agents: "No custom agents found. Create one with `/create-agent`."
- Invalid agent (missing agent.md): Show with `[INVALID: missing agent.md]` warning
- Zero invocations: Show "—" for Last Used
