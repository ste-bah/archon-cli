---
name: rollback-behavior
description: Rollback behavioral rules for an agent to a previous version. Creates a new version (never deletes history).
triggers:
  - /rollback-behavior
  - rollback behavior
arguments:
  - name: agent_name
    description: "Name of the agent, or 'global' for global rules"
    required: true
  - name: version
    description: "Version number to rollback to (optional — shows history if omitted)"
    required: false
---

# /rollback-behavior -- Revert Behavioral Rules

You are reverting behavioral rules to a previous version. Follow these steps EXACTLY.

## Step 1: Parse Input

- **agent_scope**: The agent name or "global"
- **version**: If provided, the target version number. If omitted, show version history.

Validate:
- If agent_scope != "global", verify `.claude/agents/custom/{agent_scope}/` exists
- If version provided, must be a positive integer

## Step 2: Retrieve Version History

Search MemoryGraph for all behavior rules (active AND inactive) for this scope:
- Call `mcp__memorygraph__search_memories` with tags `["behavior-rule", "{agent_scope}"]`
- Parse each result's content as JSON
- Group by `rule_group_id`
- Within each group, sort by `version` DESC

## Step 3: Display History (if no version specified)

```
## Behavior Rule History: {agent_scope}

### Rule Group: {rule_group_id}
Current: v{version} [{category}] P{priority}: {rule_text} (active={active})
  v{version-1}: {rule_text} (superseded {modified_at})
  v{version-2}: {rule_text} (superseded {modified_at})

### Rule Group: {rule_group_id}
...

To rollback: `/rollback-behavior {agent_scope} {version}`
Where {version} is the target version of a specific rule group.
Or: `/rollback-behavior {agent_scope} all:{timestamp}` to rollback ALL rules to their state at a specific time.
```

STOP and wait for user to choose a version.

## Step 4: Execute Rollback (if version specified)

1. Find the target version record in MemoryGraph for the specified rule_group_id
2. If not found: "Version {version} not found for rule group in scope '{agent_scope}'."
3. Read the target version's content (rule text, category, priority, etc.)
4. Create a NEW version (current max version + 1) with the OLD content:
   - version = max_version + 1
   - active = true
   - modified_at = now
   - modified_by = "user_request"
   - All other fields copied from the target version (rule, category, priority, source)
5. Deactivate the CURRENT active version (set active=false)
6. Create SUPERSEDES relationship: new version -> current version
7. Store in MemoryGraph

**Important**: Rollback creates a NEW version with old content. It NEVER deletes history. The SUPERSEDES chain grows.

## Step 5: Update behavior.md

If agent_scope != "global":
- Read all active rules for agent_scope from MemoryGraph
- Regenerate `.claude/agents/custom/{agent_scope}/behavior.md` with current active rules
- Sorted by priority DESC

If agent_scope == "global": skip (no agent directory for global rules)

## Step 6: Confirm

"Rolled back rule group '{rule_group_id}' to v{target_version} content (stored as v{new_version}). behavior.md updated."
