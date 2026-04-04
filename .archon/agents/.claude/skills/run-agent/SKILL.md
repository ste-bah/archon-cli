---
name: run-agent
description: Invoke a custom agent with a task description. Assembles the Context Envelope from agent definition files, memory, and LEANN, then spawns a Task tool subagent.
triggers:
  - /run-agent
  - run agent
arguments:
  - name: agent_name
    description: Name of the agent to run (must exist in .claude/agents/custom/)
    required: true
  - name: task
    description: Task description for the agent to execute
    required: true
  - name: model
    description: "Override model for the subagent (e.g., haiku, sonnet, opus). Default: inherit from parent."
    required: false
---

# /run-agent -- Execute a Custom Agent

You are assembling and executing a custom agent. Follow these steps EXACTLY, in order.

## Step 1: Parse Input

Extract from the user's command:
- **agent_name**: The agent to run (required)
- **task**: The task description (required)
- **model**: Optional model override (default: inherit parent model)

Syntax variants:
- `/run-agent {name} "task description"`
- `/run-agent {name} --model haiku "task description"`

If agent_name is missing: "Please specify an agent name. Example: `/run-agent sec-filing-analyzer 'Analyze AAPL 10-K'`"
If task is empty: "Please provide a task description. Example: `/run-agent {name} 'your task here'`"

## Step 2: Validate Agent Exists

1. Check if `.claude/agents/custom/{agent_name}/` exists using Glob or Bash `ls`
   - If not: "Agent '{agent_name}' not found. Run `/list-agents` to see available agents."
   - STOP.

2. Check if `.claude/agents/custom/{agent_name}/agent.md` exists
   - If not: "Invalid agent definition: {agent_name}/agent.md not found."
   - STOP.

## Step 3: Read Definition Files

Read each file if it exists. Track which files are present:

- `agent_md` = Read `.claude/agents/custom/{name}/agent.md` (REQUIRED)
- `context_md` = Read `.claude/agents/custom/{name}/context.md` (optional)
- `tools_md` = Read `.claude/agents/custom/{name}/tools.md` (optional)
- `behavior_md` = Read `.claude/agents/custom/{name}/behavior.md` (optional)
- `memory_keys` = Read `.claude/agents/custom/{name}/memory-keys.json` (optional, parse as JSON)
- `meta` = Read `.claude/agents/custom/{name}/meta.json` (optional, parse as JSON)

## Step 4: Recall MemoryGraph Context

If `memory_keys` exists and has `recall_queries`:
1. For each query in `recall_queries`, call `mcp__memorygraph__recall_memories` with the query
2. Collect results into `memory_context` string (concatenate relevant memory content)
3. If any query returns empty: note internally (debug level, no user warning)
4. If MemoryGraph is unavailable: set `memory_context = ""`, warn user: "MemoryGraph unavailable. Running without memory context."

## Step 5: Query LEANN Code Context

If `memory_keys` exists and has `leann_queries`:
1. Try calling `mcp__leann-search__get_stats`
   - If error/unavailable: skip ALL LEANN queries silently. Do NOT warn user.
2. If LEANN is running: for each query in `leann_queries`, call `mcp__leann-search__search_code`
3. Collect results into `leann_context` string
4. If no results or LEANN not running: set `leann_context = ""`

## Step 6: Recall and Inject Behavior Rules from MemoryGraph

This step recalls active behavior rules from MemoryGraph and combines them with behavior.md.

1. **Recall agent-scoped rules**:
   - Call `mcp__memorygraph__search_memories` with tags `["behavior-rule", "{agent_name}"]`
   - Parse each result's content as JSON
   - Filter: `active == true`

2. **Recall global rules**:
   - Call `mcp__memorygraph__search_memories` with tags `["behavior-rule", "global"]`
   - Parse each result's content as JSON
   - Filter: `active == true`

3. **If MemoryGraph unavailable**: set `mg_rules = []`, warn user: "MemoryGraph unavailable. Running with behavior.md only."

4. **Sort combined rules**:
   - Primary: priority DESC (higher number first)
   - Secondary: agent-scoped before global at same priority
   - Tertiary: most recent modified_at first
   - Quaternary: higher version first

5. **Format as markdown** for injection into the BEHAVIORAL RULES section:
   ```
   ### Rules from MemoryGraph (auto-injected)
   1. [P{priority}] [{category}] {rule_text}
   2. [P{priority}] [{category}] {rule_text}
   ...
   ```
   Store as `mg_rules_text`

6. **Reconciliation check**: If `behavior_md` exists AND `mg_rules` is non-empty:
   - Check if behavior.md content appears to diverge from MemoryGraph rules
   - Heuristic: if behavior.md contains rules not found in MemoryGraph (by substring match of rule text)
   - If diverged: warn user: "behavior.md was modified outside /adjust-behavior. File and MemoryGraph rules may be inconsistent. Run /adjust-behavior to reconcile."
   - Proceed anyway — inject both

7. **Token warning**: If combined behavior text (behavior_md + mg_rules_text) exceeds 1,500 tokens:
   - Warn: "Behavior rules consuming {N} tokens ({count} rules). Consider consolidating with /adjust-behavior."

8. **Combined behavior content** for the Context Envelope:
   ```
   {contents of behavior_md, if file exists}

   {mg_rules_text from MemoryGraph, if any rules found}
   ```

## Step 7: Assemble Context Envelope

Build the prompt string in this EXACT structure:

```
## ROLE
{contents of agent_md}

## DOMAIN CONTEXT
{contents of context_md}

## TOOL INSTRUCTIONS
{contents of tools_md}

## BEHAVIORAL RULES (auto-injected, do not override)
{combined behavior content from Step 6: behavior_md file + MemoryGraph rules}

## MEMORY CONTEXT
{memory_context from MemoryGraph}
{leann_context from LEANN}

## YOUR TASK
{task description from user}
```

Rules:
- OMIT any section that has no content (do not include empty ## headers)
- Each section is separated by a blank line
- ROLE section is ALWAYS present (agent.md is required)
- YOUR TASK section is ALWAYS present (task is required)

## Step 8: Token Budget Validation and Truncation

Estimate tokens for the assembled prompt: `ceil(total_characters / 4)`

If total controllable tokens exceed 15,000:
1. Show per-section breakdown to user:
   ```
   Token Budget Exceeded (15,000 limit):
     ROLE (agent.md):           {N} tokens  [PROTECTED]
     DOMAIN CONTEXT:            {N} tokens
     TOOL INSTRUCTIONS:         {N} tokens
     BEHAVIORAL RULES:          {N} tokens  [PROTECTED]
     MEMORY CONTEXT (MG):       {N} tokens
     MEMORY CONTEXT (LEANN):    {N} tokens
     YOUR TASK:                 {N} tokens  [PROTECTED]
     TOTAL:                     {N} tokens (over by {overage})
   ```

2. Truncate in this order:
   - First: Remove LEANN results entirely
   - Second: Remove MemoryGraph recall results entirely
   - Third: Truncate context.md from the end, add `[TRUNCATED]` marker
   - NEVER truncate: agent.md, behavior.md, task description

3. Recalculate total after truncation.

## Step 9: Spawn Subagent (NO confirmation needed — the user's command IS the intent)

Immediately spawn the agent. Do NOT ask "proceed?" or show token breakdowns. Just run it.

1. Use the Agent tool (Task tool) with the assembled Context Envelope as the prompt
2. If `--model` was specified, set the model parameter accordingly
3. Wait for the subagent to complete
4. Display the subagent's output to the user

On error (timeout, crash):
- Report error: "Agent '{name}' execution failed: {error}"
- Do NOT update invocation count
- Do NOT write trace file
- STOP.

## Step 10b: Write Execution Trace (Autolearn — REQ-LEARN-001)

After the subagent returns successfully:

1. **Determine task_completed_heuristic**: Set to `true` if the Task tool returned output without error. Set to `false` if the Task tool errored or timed out. This is a heuristic — TASK-AGT-014's analysis may override it.

2. **Create trace directory** if it doesn't exist:
   ```bash
   mkdir -p .claude/agents/traces/{agent_name}/
   ```

3. **Generate timestamp filename**: Replace colons with hyphens for filesystem safety:
   `2026-03-30T10-00-00.json`

4. **Write trace file** to `.claude/agents/traces/{agent_name}/{timestamp}.json`:
   ```json
   {
     "schema_version": 1,
     "agent_name": "{agent_name}",
     "task_description": "{task from user}",
     "context_envelope": "{full assembled prompt from Step 7}",
     "agent_output": "{full Task tool return value}",
     "task_completed_heuristic": true,
     "user_feedback": null,
     "model": "{model used}",
     "timestamp": "{ISO 8601 UTC}",
     "duration_ms": 0,
     "invocation_number": {current invocation_count + 1}
   }
   ```
   Note: Truncate `context_envelope` and `agent_output` to 50,000 chars each if longer (append `[TRUNCATED]`).

5. **Store MemoryGraph execution record** (lightweight reference):
   ```
   mcp__memorygraph__store_memory:
     type: "general"
     title: "exec_{agent_name}_{timestamp}"
     content: "Agent '{agent_name}' invocation #{invocation_number}. Task: {first 100 chars}. Completed: {heuristic}. Trace: .claude/agents/traces/{agent_name}/{timestamp}.json"
     tags: ["agent-execution", "{agent_name}"]
     importance: 0.3
   ```
   If MemoryGraph unavailable: skip silently (trace file on disk is sufficient).

## Step 11: Update meta.json (Write 1 — immediate)

After successful execution:
1. Read current `meta.json` from `.claude/agents/custom/{name}/meta.json`
2. Parse as JSON
3. Increment `invocation_count` by 1
4. Update `last_used` to current ISO 8601 timestamp (UTC)
5. Update `quality.total_selections` by 1 (if quality block exists)
6. Write the updated JSON to `meta.json.tmp` in the same directory
7. Read back to verify it's valid JSON
8. Use Bash to rename: `mv .claude/agents/custom/{name}/meta.json.tmp .claude/agents/custom/{name}/meta.json`

If `meta.json` does not exist, create it with all fields:
```json
{
  "created": "{current timestamp}",
  "last_used": "{current timestamp}",
  "version": 1,
  "generation": 0,
  "author": "user",
  "invocation_count": 1,
  "quality": {
    "total_selections": 1,
    "total_completions": 0,
    "total_fallbacks": 0,
    "applied_rate": 0.0,
    "completion_rate": 0.0,
    "effective_rate": 0.0,
    "fallback_rate": 0.0
  },
  "evolution_history_last_10": []
}
```

## Step 12: Post-Execution Summary (Optional)

If MemoryGraph is available, store a brief output summary:
- Call `mcp__memorygraph__store_memory` with:
  - type: "general"
  - title: "Agent output: {name} -- {first 50 chars of task}"
  - content: "Agent '{name}' executed task: {first 100 chars of task}. Output summary: {first 200 chars of output}."
  - tags: ["agent-output", "{name}"]
  - importance: 0.5

If MemoryGraph is unavailable, skip this step silently.

## Step 13: User Feedback Prompt (Autolearn — REQ-LEARN-004)

After displaying the agent output, ask a structured feedback question:

```
Agent output above. Quick feedback? (good / bad / skip)
```

- "good", "great", "perfect", "yes" -> user_feedback = "positive"
- "bad", "wrong", "incorrect", "no" -> user_feedback = "negative"; override task_completed_heuristic to false in trace
- Free-text correction -> user_feedback = user's message; override heuristic to false
- "skip", "", or command starting with "/" -> user_feedback = null

If feedback provided, update the trace file:
1. Read trace JSON from `.claude/agents/traces/{agent_name}/{timestamp}.json`
2. Set `user_feedback` field
3. If negative: set `task_completed_heuristic` to false
4. Atomic rewrite

## Step 14: Post-Task Analysis (Autolearn — REQ-LEARN-003)

**Skip if**: `invocation_count < 3` (warm-up exclusion — first 2 invocations are not analyzed)

This step runs INLINE (synchronous) — the user sees a brief pause (~2-5s) while Haiku analyzes.

1. **Read trace file** from disk (the one written in Step 10b)
2. **Truncate for analysis prompt**: context_envelope to 12,000 chars, agent_output to 8,000 chars
3. **Retrieve recent analyses** from MemoryGraph: search for tag `["agent-analysis", "{agent_name}"]`, take last 3
4. **Spawn Haiku analysis subagent** (prompt-in/JSON-out, no tool access):

```
You are an agent execution analyst. Read the execution trace and produce a structured judgment.

## Agent Definition Summary
{first 500 chars of agent.md}

## Context Envelope (truncated)
{context_envelope, truncated to 12K chars}

## Agent Output (truncated)
{agent_output, truncated to 8K chars}

## User Feedback
{user_feedback or "No user feedback provided."}

## Agent Quality History
Total invocations: {total_selections}
Completion rate: {completion_rate}
Recent issues (last 3): {issues from previous analyses or "None"}

Produce ONLY valid JSON matching this schema:
{
  "task_completed": boolean,
  "completion_quality": "full" | "partial" | "failed",
  "agent_followed_role": boolean,
  "agent_followed_behavior_rules": boolean,
  "issues_identified": ["string per issue"],
  "evolution_suggestions": [
    {"type": "FIX"|"DERIVED"|"CAPTURED", "target_file": "context.md"|"agent.md"|"tools.md", "direction": "what to change and why"}
  ]
}
```

5. **Parse Haiku output**: Try JSON.parse; if fails, extract with regex `/{[\s\S]*}/`; if still fails, mark as unanalyzed
6. **Reclassify behavior.md targets**: If any suggestion has `target_file: "behavior.md"`, reclassify as a behavior adjustment proposal (not a FIX evolution)
7. **Store analysis in MemoryGraph**:
   ```
   mcp__memorygraph__store_memory:
     type: "general"
     title: "analysis_{agent_name}_{timestamp}"
     content: {full analysis JSON}
     tags: ["agent-analysis", "{agent_name}"]
     importance: 0.4
   ```
8. **Write 2 to meta.json** (quality counter update with authoritative task_completed):
   - Read meta.json
   - If analysis says `task_completed: true`: increment `quality.total_completions`
   - If analysis says `task_completed: false`: increment `quality.total_fallbacks`
   - Recalculate rates: `completion_rate = completions / selections`, `effective_rate = completions / selections`, `fallback_rate = fallbacks / selections`
   - Set `last_analysis_invocation = invocation_count` (for detecting unanalyzed invocations)
   - Atomic write

9. **Show evolution suggestions to user** (if any):
   ```
   Analysis complete. {N} issue(s) identified, {M} evolution suggestion(s).
   {If suggestions: "Run /evolve-agent {agent_name} to review and apply suggestions."}
   ```

## Step 15: Periodic Health Check (Autolearn — REQ-LEARN-009, every 5 invocations)

**Skip if**: `invocation_count` is not a multiple of 5 (only check at 5, 10, 15, ...)
**Skip if**: `quality.total_selections < 5` (not enough data)

Check quality thresholds:
- If `fallback_rate > 0.40`: flag
- If `completion_rate < 0.35`: flag
- If `effective_rate < 0.55`: flag

If any threshold breached:
```
Health Check: Agent '{agent_name}' is underperforming.
  effective_rate: {rate} (threshold: > 0.55)
  completion_rate: {rate} (threshold: > 0.35)
  fallback_rate: {rate} (threshold: < 0.40)
  invocations: {count}

Options:
1. Run `/evolve-agent {agent_name}` to review and apply improvement suggestions
2. Run `/agent-history {agent_name}` to rollback to a previous version
3. Ignore (agent may still be learning)
```

Special case (EC-LEARN-008): If `total_completions == 0` (agent has NEVER succeeded):
```
Agent '{agent_name}' has never succeeded (0/{count} invocations).
Consider:
1. `/evolve-agent {agent_name}` — review what's going wrong
2. Delete and recreate with `/create-agent`
3. Manually edit files in `.claude/agents/custom/{agent_name}/`
```

**Anti-loop guards** (REQ-LEARN-006):
- Max 1 evolution per invocation session (tracked in-memory — if `/evolve-agent` was already called, do not suggest again)
- After any evolution, quality counters reset to 0 — the agent needs 5+ fresh invocations before re-evaluation
- LLM confirmation gate (REQ-LEARN-014): Before executing evolution from Trigger 2/3 (not post-task), ask Haiku to confirm the evolution is warranted based on quality metrics + recent analyses. If Haiku says no, skip and log.
