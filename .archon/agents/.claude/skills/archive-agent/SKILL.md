---
name: archive-agent
description: Archive a custom agent (move to archived directory) or restore a previously archived agent.
triggers:
  - /archive-agent
  - /restore-agent
  - archive agent
  - restore agent
arguments:
  - name: agent_name
    description: Name of the agent to archive or restore
    required: true
---

# /archive-agent and /restore-agent -- Agent Lifecycle Management

## /archive-agent {name}

### Step 1: Validate
- Check `.claude/agents/custom/{name}/` exists
- If not: "Agent '{name}' not found."

### Step 2: Warn if Recently Used
- Read `meta.json` for `last_used` timestamp
- If last used within 60 seconds: "Agent '{name}' was used recently. Archive anyway?"
- Wait for confirmation

### Step 3: Move Directory
```bash
mkdir -p .claude/agents/archived/
mv .claude/agents/custom/{name}/ .claude/agents/archived/{name}/
```

### Step 4: Update MemoryGraph
- Search for agent definition memory with tag `["agent-definition", "{name}"]`
- If found: update to add tag `"archived"`

### Step 5: Confirm
"Agent '{name}' archived. Restore with: `/restore-agent {name}`"

---

## /restore-agent {name}

### Step 1: Validate
- Check `.claude/agents/archived/{name}/` exists
- If not: "Archived agent '{name}' not found."
- Check `.claude/agents/custom/{name}/` does NOT exist (name collision)
- If exists: "Agent '{name}' already exists in custom/. Remove it first or use a different name."

### Step 2: Move Directory
```bash
mv .claude/agents/archived/{name}/ .claude/agents/custom/{name}/
```

### Step 3: Update MemoryGraph
- Search for agent definition memory with tags `["agent-definition", "{name}", "archived"]`
- If found: remove `"archived"` tag

### Step 4: Confirm
"Agent '{name}' restored. Run with: `/run-agent {name} \"task\"`"
