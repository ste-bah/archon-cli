---
name: create-agent
description: Create a custom agent from a natural language description. Generates agent definition files (agent.md, context.md, tools.md, behavior.md, memory-keys.json, meta.json) in .claude/agents/custom/{name}/.
triggers:
  - /create-agent
  - create agent
  - make an agent
  - new agent
arguments:
  - name: description
    description: Natural language description of the agent's purpose and capabilities
    required: true
  - name: name
    description: Optional explicit agent name (will be sanitized to lowercase-hyphenated)
    required: false
---

# /create-agent -- Generate a Custom Agent Definition

You are creating a custom agent definition based on the user's description. Follow these steps EXACTLY.

## Step 1: Parse Input

Extract from the user's command:
- **description**: The natural language description of the agent
- **name** (optional): If the user provided `--name "..."`, use it. Otherwise, derive a name from the description.

If the description is empty or too vague (fewer than 10 words, no specific domain or task mentioned):
- Respond with: "Please provide a more specific description. Example: `/create-agent 'Analyzes SEC 10-K filings for revenue recognition risks'`"
- STOP. Do not proceed.

## Step 2: Sanitize Name

Derive or sanitize the agent name:
1. If no explicit name: extract 2-4 key words from the description
2. Apply sanitization: lowercase, replace spaces/underscores with hyphens, strip special chars, strip leading digits, collapse consecutive hyphens, remove leading/trailing hyphens
3. Max length: 50 characters
4. Verify the name matches pattern: `^[a-z][a-z0-9-]*[a-z0-9]$`
5. Verify the name is not reserved: `_template`, `archived`, `versions`, `traces`, `custom`

If sanitization fails, ask the user for an explicit name.

## Step 3: Check for Collisions

1. Check if `.claude/agents/custom/{name}/` already exists
   - If yes: "Agent '{name}' already exists. Use a different name or manually edit files in `.claude/agents/custom/{name}/`."
   - STOP.

2. Search for overlap with existing agents:
   - Use Glob to read directory names in `.claude/agents/custom/`
   - If any existing name is a substring of the new name (or vice versa): warn
   - "Existing agent '{existing}' has a similar name. Create anyway?"

3. Search MemoryGraph for existing agent registrations:
   - Use `mcp__memorygraph__recall_memories` with query "agent-definition {description keywords}"
   - If matching agent-definition memories found: warn about overlap

If any warnings were raised, wait for user to confirm before proceeding.

## Step 4: Generate Agent Definition

Based on the description, generate the following files. OMIT any file that would be empty or contain only generic boilerplate with no domain-specific content. Only `agent.md` and `meta.json` are always created.

### agent.md (ALWAYS generated -- Master Prompt Framework)

Apply the 12 Principles from the AI Agent Prompt Guide internally:
1. Specificity: Every capability must be concrete, not vague
2. Harm Prohibition: FORBIDDEN OUTCOMES must list what the agent CANNOT do
3. Scope Anchoring: SCOPE must have both In Scope and Out of Scope
4. Intent Declaration: INTENT must state the goal AND the value
5. Negative Space: Out of Scope + FORBIDDEN cover what is NOT wanted
6. Preserve Behavior: CONSTRAINTS prevent overreach
7. Quality Requirements: OUTPUT FORMAT defines what "done" looks like
8. Environmental Constraints: CONSTRAINTS include technical limits (depth=1)
9. Reversibility: WHEN IN DOUBT prefers conservative interpretations
10. Source Specification: CONSTRAINTS specify data sources when relevant
11. Cascade Prevention: Out of Scope prevents scope creep
12. Good Faith: WHEN IN DOUBT is the catch-all for ambiguity

Structure:
```
# {Agent Name -- Title Case}

## INTENT
{What this agent does and WHY it exists. 2-3 sentences.}

## SCOPE
### In Scope
- {Capability 1}: {specific description}
- {Capability 2}: {specific description}
- {Capability 3+}: {as needed}

### Out of Scope
- {What this agent explicitly does NOT do}
- {Tasks that should be done by other agents or manually}

## CONSTRAINTS
- You run at depth=1 and CANNOT spawn subagents or use the Task/Agent tool
- You MUST complete your task directly using the tools available to you
- {Domain-specific constraint 1}
- {Domain-specific constraint 2}

## FORBIDDEN OUTCOMES
- DO NOT {specific prohibited behavior 1}
- DO NOT {specific prohibited behavior 2}
- DO NOT fabricate data or present assumptions as facts
- DO NOT echo user-provided input in error messages (XSS prevention)
- {Domain-specific prohibition}

## EDGE CASES
- {Edge case 1}: {expected behavior}
- {Edge case 2}: {expected behavior}
- {Edge case 3}: {expected behavior}

## OUTPUT FORMAT
{Structured output format appropriate to the agent's role:
- For analyzers: Summary + Findings (numbered, with severity/confidence) + Key Metrics + Assessment
- For coders: Plan + Implementation + Tests + Self-Review
- For writers: Draft + Structure Notes + Missing Information
- For reviewers: Issues (with severity) + Recommendations + Summary}

## WHEN IN DOUBT
If any part of the task is ambiguous, choose the interpretation that:
1. Is most conservative / least risky
2. Follows existing patterns in the codebase or domain
3. Produces verifiable output with citations or references
If still uncertain, state the ambiguity explicitly in your output.
```

Token budget: target 1,000-3,000 tokens. Hard limit: 3,000 tokens.

### context.md (generate only if description implies domain knowledge)
```
# Domain Context

## Background
{Domain-specific background the agent needs.}

## Key Concepts
- **{Concept}**: {Definition}

## Reference Data
{Schemas, formats, conventions.}

## Common Patterns
{Domain patterns to follow.}
```
Token budget: target 1,000-5,000 tokens. Hard limit: 5,000 tokens.

### tools.md (generate only if description implies specific tool usage)
```
# Tool Instructions

## Primary Tools
{Which tools are most relevant and how to use them.}

## Domain-Specific Patterns
{Tool usage patterns specific to this agent's task.}
```
Token budget: target 500-2,000 tokens. Hard limit: 2,000 tokens.

### behavior.md (generate only if description implies behavioral constraints)
```
# Behavioral Rules

## Communication
{How the agent should communicate findings.}

## Quality Standards
{Verification and accuracy requirements.}

## Process
{Step-by-step approach the agent should follow.}
```
Token budget: target 500-1,500 tokens. Hard limit: 1,500 tokens.

### memory-keys.json (generate with suggested keys based on domain)
```json
{
  "recall_queries": ["{suggested MemoryGraph keys based on domain}"],
  "leann_queries": ["{suggested code search queries if agent works with code}"],
  "tags": ["agent-definition", "{domain-tag}"]
}
```

### meta.json (ALWAYS generated)
```json
{
  "created": "{current ISO 8601 timestamp}",
  "last_used": "{current ISO 8601 timestamp}",
  "version": 1,
  "generation": 0,
  "author": "user",
  "invocation_count": 0,
  "quality": {
    "total_selections": 0,
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

## Step 5: Depth=1 Validation

Scan the generated `agent.md` and `tools.md` (if generated) for patterns that imply subagent spawning:
- `Task(`
- `spawn agent`
- `delegate to sub-agent`
- `use the Agent tool`
- `create a subagent`
- `spawn a worker`

If any pattern is found:
- Warn: "Custom agents run at depth=1 and cannot spawn subagents. The following pattern was found: '{pattern}' in {file}. Consider rephrasing the agent's role to perform tasks directly."
- Suggest a rewrite that removes the subagent dependency.

## Step 6: Token Budget Validation

For each generated markdown file, estimate tokens using `ceil(length / 4)`:
- `agent.md`: hard limit 3,000 tokens
- `context.md`: hard limit 5,000 tokens
- `tools.md`: hard limit 2,000 tokens
- `behavior.md`: hard limit 1,500 tokens
- Total controllable: hard limit 15,000 tokens

If any file exceeds its hard limit, warn with the specific count and suggest which sections to trim.

## Step 7: Write Files (NO approval needed — the user's command IS the intent)

Do NOT present the definition for review. Do NOT ask "approve/revise/cancel". Just create the files immediately. The user asked to create an agent — that IS the approval.

1. Create directory: `.claude/agents/custom/{name}/`
2. Write each generated file using the Write tool
3. Register in MemoryGraph:
   ```
   mcp__memorygraph__store_memory:
     type: "general"
     title: "Agent definition: {name}"
     content: "Custom agent definition: {name} -- {one-line description from INTENT section}"
     tags: ["agent-definition", "{name}"]
     importance: 0.7
   ```
4. Confirm: "Agent '{name}' created successfully. Run it with: `/run-agent {name} \"your task here\"`"

On "revise":
- Ask what to change
- Regenerate affected files
- Return to Step 7

On "cancel":
- Confirm: "Agent creation cancelled."
- STOP.
