---
name: evolve-agent
description: Review and apply evolution suggestions for a custom agent. Supports FIX (in-place repair), DERIVED (create variant), and CAPTURED (extract from success).
triggers:
  - /evolve-agent
  - evolve agent
arguments:
  - name: agent_name
    description: Name of the agent to evolve
    required: true
---

# /evolve-agent -- Review and Apply Evolution Suggestions

## Step 1: Retrieve Recent Analyses

1. Search MemoryGraph for analyses: tags `["agent-analysis", "{agent_name}"]`
2. Sort by timestamp DESC, take last 5
3. Collect all `evolution_suggestions` from these analyses
4. Deduplicate by direction text (same suggestion from multiple analyses = one entry)
5. If no suggestions: "No evolution suggestions for '{agent_name}'. The agent is performing well or needs more invocations for analysis."

## Step 2: Present Suggestions

```
## Evolution Suggestions for {agent_name}

Quality: effective_rate={rate}, invocations={count}

### Suggestion 1: [{type}] {target_file}
Direction: {direction}
From analysis: {analysis_timestamp}
Analyses supporting this: {count}

### Suggestion 2: [{type}] {target_file}
...

Actions:
1. Apply suggestion #{N}
2. Apply all FIX suggestions
3. Skip / dismiss all
```

Wait for user choice.

## Step 3: Execute FIX Evolution

For each approved FIX suggestion:

1. **Read current file**: Read `.claude/agents/custom/{agent_name}/{target_file}`
2. **Read recent traces**: Last 3 trace files from `.claude/agents/traces/{agent_name}/`
3. **Spawn Haiku subagent** for edit generation (prompt-in/JSON-out, no tool access):

```
You are an agent definition editor. Given the current file content, an analysis of what went wrong, and recent execution traces, produce targeted SEARCH/REPLACE edits.

## Current Content of {target_file}
{file content}

## Issue Identified
{direction from evolution suggestion}

## Recent Execution Context
{last 3 trace summaries, truncated}

Produce ONLY a JSON array of edits:
[
  {"search": "exact text to find", "replace": "replacement text"},
  {"search": "...", "replace": "..."}
]

Rules:
- The "search" text MUST appear verbatim in the current file
- Make minimal, targeted changes — do not rewrite unrelated sections
- Do NOT modify CONSTRAINTS or FORBIDDEN OUTCOMES sections unless the issue is specifically about them
- FIX must NOT target behavior.md — behavioral changes go through /adjust-behavior
```

4. **Parse edits**: Extract JSON array from Haiku response
5. **Show diff to user**:
   ```
   ## Proposed FIX for {target_file}

   Edit 1:
   - {search text}
   + {replace text}

   Edit 2:
   ...

   Approve? (yes / no / modify)
   ```

6. **On approval**:
   - Backup current files to `.claude/agents/versions/{agent_name}/{version}/`
   - Apply each edit using the Edit tool
   - Update meta.json: version++, generation++, append to evolution_history_last_10
   - Reset quality counters: total_selections=0, total_completions=0, total_fallbacks=0, all rates=0.0
   - Create MemoryGraph version record:
     ```
     mcp__memorygraph__store_memory:
       type: "general"
       title: "agent_version:{agent_name}:v{version}"
       content: {"version": N, "generation": M, "evolution_type": "FIX", "change_summary": "{direction}", "trigger": "post-task-analysis"}
       tags: ["agent-version", "{agent_name}"]
     ```
   - Create EVOLVED_FROM relationship to previous version
   - Confirm: "Agent '{agent_name}' evolved: FIX applied to {target_file} (v{old}->v{new})"

7. **Anti-loop guard**: Max 1 evolution per invocation. If already evolved in this session, queue remaining suggestions for next `/evolve-agent` call.

## Step 4: Execute DERIVED Evolution

For DERIVED suggestions:

1. Generate new agent name: `{agent_name}-v{generation+1}` (check for collisions)
2. Copy parent agent directory to new directory
3. Apply modifications from the suggestion direction
4. Set meta.json: `parent_agent: "{agent_name}"`, `evolution_type: "DERIVED"`, version=1, generation=parent.generation+1
5. Register new agent in MemoryGraph
6. Both parent and derived agent remain active
7. Confirm: "Derived agent '{new_name}' created from '{agent_name}'"

## Step 5: Execute CAPTURED Evolution

For CAPTURED suggestions (typically from two-phase fallback):

1. Read the successful bare-reasoning trace
2. Spawn Sonnet subagent to extract agent definition from the trace
3. Generate agent.md + context.md from the extraction
4. Present to user for approval (same flow as /create-agent Step 7)
5. On approval: create new agent directory, register in MemoryGraph
6. Confirm: "Captured agent '{name}' created from successful execution"
