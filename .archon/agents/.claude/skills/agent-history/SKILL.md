---
name: agent-history
description: Show the version history and evolution lineage for a custom agent. Supports rollback to any previous version.
triggers:
  - /agent-history
  - agent history
arguments:
  - name: agent_name
    description: Name of the agent to show history for
    required: true
---

# /agent-history -- Version History and Evolution Lineage

## Step 1: Retrieve Version Records

1. Search MemoryGraph for version records: tags `["agent-version", "{agent_name}"]`
2. Sort by version DESC (most recent first)
3. Also read current `meta.json` for quality counters

## Step 2: Display Version DAG

```
## Agent History: {agent_name}

Current: v{version} (generation {generation})
Quality: effective_rate={rate}, invocations={count}, completions={completions}

### Version Timeline
v{N} [CURRENT] ({evolution_type}) — {date}
  Summary: {change_summary}
  Trigger: {trigger}
  Quality at this version: effective_rate={rate}

v{N-1} ({evolution_type}) — {date}
  Summary: {change_summary}
  Trigger: {trigger}

v{N-2} ({evolution_type}) — {date}
  Summary: {change_summary}
  Trigger: {trigger}

...

v1 (CREATED) — {date}
  Original agent definition

### Actions
1. View diff for version #{N}: `/agent-history {agent_name} --diff {N}`
2. Rollback to version #{N}: `/agent-history {agent_name} --rollback {N}`
```

## Step 3: View Diff (if --diff flag)

1. Find the version record in MemoryGraph
2. Display the `content_diff` field
3. If content_diff is empty (e.g., DERIVED with multi-parent): "No diff available for this version type."

## Step 4: Rollback (if --rollback flag)

1. Find the target version's snapshot on disk: `.claude/agents/versions/{agent_name}/{version}/`
2. If snapshot not found: "Version {N} snapshot not found on disk. Only the last 5 versions are retained."
3. Show current vs target file comparison
4. On approval:
   - Backup current files to `.claude/agents/versions/{agent_name}/{current_version}/`
   - Copy target version files to `.claude/agents/custom/{agent_name}/`
   - Create new version record: type=ROLLBACK, version=max+1, generation unchanged
   - Reset quality counters
   - Update meta.json
   - Confirm: "Rolled back to v{target} content (stored as v{new})"
