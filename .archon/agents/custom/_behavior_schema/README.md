# Behavior Rule MemoryGraph Schema — v1.0.0

Reference document for skills that manage behavior rules. This is NOT executable code — it defines the conventions that `/adjust-behavior`, `/run-agent`, and `/rollback-behavior` follow.

## Node Schema

Each behavior rule is stored as a MemoryGraph memory:

```
title:    behavior_rule:{agent_scope}:{category}:{slug}:v{version}
type:     general  (NOTE: MemoryGraph only supports fixed types; we use "general"
                    and discriminate via the "behavior-rule" tag)
tags:     ["behavior-rule", "{agent_scope}", "{category}"]
content:  JSON string of the metadata below (MemoryGraph stores content as string)
context:
  schema_version: "1.0.0"
  category:       communication | coding | delegation | analysis | quality
  rule:           "The actual rule text" (max 500 chars)
  priority:       1-100 (higher = more important, default 50)
  source:         user_request | auto_extracted | correction
  version:        integer >= 1 (per rule_group_id)
  active:         true | false
  agent_scope:    "global" | "{agent-name}"
  rule_group_id:  "brg-{8 hex chars}" (groups all versions of one logical rule)
  created_at:     ISO 8601 UTC (when v1 was first created)
  modified_at:    ISO 8601 UTC (when this version was stored)
  modified_by:    user_request | auto_extracted | correction
```

## Slug Generation

```
slug = rule_text[:40].lower()
slug = re.sub(r'[^a-z0-9]+', '-', slug).strip('-') or "unnamed-rule"
```

## Priority Semantics

- Range: 1-100 (integer)
- Higher number = higher priority
- Tiebreaker when same priority:
  1. Most recent modified_at wins
  2. If same modified_at: higher version wins
- Agent-specific rules appear before global rules at the same priority level

## Relationships

- **SUPERSEDES** (new_version -> old_version): Created on update. Linear chain per rule_group_id.
- **CONFLICTS_WITH** (rule_a <-> rule_b): Created when /adjust-behavior detects conflict. Bidirectional. Advisory only.
- **APPLIES_TO** (rule -> agent_definition): Created for agent-scoped rules (not global).

## CRUD Operations

### create_behavior_rule
1. Validate: rule non-empty <= 500 chars, category valid, priority 1-100, source valid
2. Generate rule_group_id: "brg-" + 8 hex chars
3. Generate slug from rule text
4. Store via mcp__memorygraph__store_memory with full metadata
5. If agent-scoped: create APPLIES_TO relationship

### read_active_rules_for_agent(agent_name)
1. Search MemoryGraph for active behavior_rule with agent_scope = agent_name
2. Search MemoryGraph for active behavior_rule with agent_scope = "global"
3. Merge, sort by: priority DESC, modified_at DESC, version DESC
4. Agent-scoped before global at same priority

### update_behavior_rule
1. Mark current version as active=false
2. Create new version (version + 1) with updated fields
3. Create SUPERSEDES relationship: new -> old
4. If conflict detected: create CONFLICTS_WITH relationship

### deactivate_behavior_rule
1. Set active=false on the current version
2. Do NOT delete the node (soft delete preserves history)
