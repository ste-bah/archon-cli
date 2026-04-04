---
name: adjust-behavior
description: Adjust behavioral rules for a custom agent or globally. Uses LLM-mediated merging with diff validation to prevent hallucinated rule changes.
triggers:
  - /adjust-behavior
  - adjust behavior
  - change behavior
  - add behavior rule
arguments:
  - name: agent_name
    description: "Name of the agent to adjust, or 'global' for global rules. If omitted, defaults to global."
    required: false
    default: "global"
  - name: adjustment
    description: "Natural language description of the behavior change"
    required: true
---

# /adjust-behavior -- Adjust Behavioral Rules

You are adjusting behavioral rules for a custom agent (or globally). Follow these steps EXACTLY.

## Schema Reference

Behavior rules are stored in MemoryGraph as `type: "general"` with tag `"behavior-rule"`.
See `.claude/agents/custom/_behavior_schema/README.md` for the full schema.

Key fields in the content JSON: schema_version, category, rule, priority (1-100), source, version, active, agent_scope, rule_group_id, created_at, modified_at.

## Step 1: Parse Input

Extract:
- **agent_scope**: If first arg matches an existing agent in `.claude/agents/custom/`, use it. Otherwise, "global".
- **adjustment**: The behavior change description (required, max 1000 chars).

Validation:
- If adjustment is empty: "Please describe the behavior change. Example: `/adjust-behavior sec-filing-analyzer 'Always cite specific section numbers'`"
- If agent_scope is not "global", verify `.claude/agents/custom/{agent_scope}/` exists. If not: "Agent '{agent_scope}' not found."
- If adjustment > 1000 chars: "Adjustment text too long (max 1000 characters)."

## Step 2: Recall Current Rules

1. Search MemoryGraph for active rules matching the scope:
   - Call `mcp__memorygraph__search_memories` with tags `["behavior-rule", "{agent_scope}"]`
   - Parse each result's content as JSON
   - Filter: `active == true`
   - Sort by: priority DESC, then modified_at DESC
2. If agent_scope != "global", also recall global rules (for context only, not editable):
   - Call `mcp__memorygraph__search_memories` with tags `["behavior-rule", "global"]`
   - Filter: `active == true`
3. If MemoryGraph is unavailable: warn user, proceed with empty rule set.

## Step 3: Format Current Rules as Markdown

```
## Current Rules for {agent_scope}

### Agent-Scoped Rules
1. [P{priority}] [{category}] {rule_text}  (id: {rule_group_id}, v{version})
2. ...

### Global Rules (read-only context, do not modify)
1. [P{priority}] [{category}] {rule_text}
2. ...
```

If no rules exist: "No existing rules."

## Step 4: LLM Merge via Haiku Subagent

Spawn a Task subagent with model haiku:

**Prompt for the merge subagent:**

```
You are a behavior rule merge assistant. Integrate a new behavior adjustment into an existing set of rules.

RULES:
1. Preserve ALL existing rules unless the adjustment explicitly contradicts or replaces one.
2. If the adjustment contradicts an existing rule, mark the old rule as REMOVED and add the new rule.
3. If the adjustment modifies an existing rule, mark it as MODIFIED and provide updated text.
4. If the adjustment adds new behavior, add it as NEW.
5. Assign category: communication, coding, delegation, analysis, or quality.
6. Assign priority 1-100: "always"/"never"/"critical" = 75-90; "prefer"/"try to" = 30-40; default = 50.
7. Do NOT invent, remove, or rephrase rules unrelated to the adjustment.
8. Do NOT modify global rules — they are read-only context.
9. Output ONLY valid JSON. No markdown fences, no explanation.

OUTPUT FORMAT:
{"merged_rules": [{"rule_group_id": "existing-id or null for new", "category": "coding", "rule": "The rule text", "priority": 75, "status": "UNCHANGED|MODIFIED|NEW|REMOVED", "original_rule": "original text if MODIFIED/REMOVED, null otherwise", "change_reason": "explanation for MODIFIED/NEW/REMOVED only"}]}

## Current Rules
{current_rules_markdown from Step 3}

## Adjustment Request
{adjustment from user}

## Agent Scope
{agent_scope}

Produce the merged ruleset as JSON.
```

## Step 5: Parse Haiku Output

1. Try `JSON.parse` on the full response
2. If fails: try regex to extract JSON block (`/{[\s\S]*}/`)
3. If fails: "Merge failed: could not parse output as JSON. Please try again." STOP.
4. Validate: `merged_rules` is an array, each entry has required fields (rule, category, priority, status)
5. If validation fails: "Merge produced invalid output. Please try again." STOP.

## Step 6: Diff Validation

Compare merged output against existing rules to catch hallucinations:

**Check 1 — Silently removed rules:**
For each existing rule whose rule_group_id does NOT appear in merged_rules (or appears with status != "REMOVED"):
- FLAG as "SILENTLY_REMOVED" (severity: error) — rule disappeared without explanation

**Check 2 — Hallucinated new rules:**
For each merged rule with status "NEW" and null rule_group_id:
- Extract significant words from the adjustment text (exclude: the, a, an, is, are, to, for, of, in, on, with, and, or, not, it, this, that, be, do)
- Extract significant words from the new rule
- If zero word overlap: FLAG as "HALLUCINATED_NEW" (severity: error) — rule unrelated to adjustment

**Check 3 — Modified rules with no actual change:**
For each merged rule with status "MODIFIED":
- Compare original_rule with rule text
- If identical after strip: FLAG as "COSMETIC_ONLY" (severity: info)

**Check 4 — Priority changes without explanation:**
For each merged rule that changed priority vs existing:
- If no change_reason provided: FLAG as "UNEXPLAINED_PRIORITY_CHANGE" (severity: warning)

## Step 7: Present to User

Display the merged result with diff annotations:

```
## Behavior Adjustment: {agent_scope}

### Changes:
{For each rule with status != UNCHANGED:}
  - [{status}] [{category}] P{priority}: {rule_text}
    {If MODIFIED: "Was: {original_rule}"}
    {If REMOVED: "Reason: {change_reason}"}
    {If NEW: "Reason: {change_reason}"}

### Unchanged Rules:
{For each rule with status == UNCHANGED:}
  - [{category}] P{priority}: {rule_text}

### Flags:
{For each FLAG from Step 6:}
  - [{severity}] {type}: {description}

Would you like to:
1. **Approve** -- Apply these changes
2. **Revise** -- Tell me what to change
3. **Cancel** -- Discard everything
```

WAIT for explicit user approval.

## Step 8: Store on Approval

On approval, for each merged rule:

**UNCHANGED rules**: No action.

**NEW rules**:
1. Generate rule_group_id: "brg-" + 8 random hex chars
2. Generate slug from rule text
3. Store via `mcp__memorygraph__store_memory`:
   - type: "general"
   - title: `behavior_rule:{agent_scope}:{category}:{slug}:v1`
   - content: JSON with all metadata fields (version=1, active=true)
   - tags: ["behavior-rule", agent_scope, category]
4. If agent_scope != "global": create APPLIES_TO relationship to agent definition

**MODIFIED rules**:
1. Find existing rule's memory ID via search
2. Update existing rule: set active=false via `mcp__memorygraph__update_memory`
3. Create new version: version+1, new content with updated rule text
4. Store new version via `mcp__memorygraph__store_memory`
5. Create SUPERSEDES relationship: new version -> old version via `mcp__memorygraph__create_relationship`

**REMOVED rules**:
1. Find existing rule's memory ID
2. Update: set active=false via `mcp__memorygraph__update_memory`
3. No new version created — just deactivated

**After all changes applied:**
- If agent_scope != "global", regenerate `.claude/agents/custom/{agent_scope}/behavior.md`:
  - Read all active rules for agent_scope
  - Format as markdown:
    ```
    # Behavioral Rules
    # Auto-generated by /adjust-behavior -- do not edit manually
    # Source of truth: MemoryGraph behavior_rule nodes
    # Last updated: {now}

    ## Rules (sorted by priority)
    1. **[P{priority}]** [{category}] {rule_text}
    2. ...
    ```
  - Write to `.claude/agents/custom/{agent_scope}/behavior.md`
- If agent_scope == "global": SKIP behavior.md regeneration (global rules have no agent directory)

Confirm: "Behavior rules updated for '{agent_scope}'. {N} rules changed ({n_new} new, {n_modified} modified, {n_removed} removed)."

On "revise": ask what to change, re-run Step 4 with feedback.
On "cancel": "Behavior adjustment cancelled."
