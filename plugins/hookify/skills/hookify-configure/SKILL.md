---
name: hookify-configure
description: Enable or disable existing hookify rules interactively; use when the user wants to toggle, turn on, or turn off hookify rules in the current project.
license-source: https://github.com/ste-bah/test-plugins-official/tree/main/plugins/hookify (Apache-2.0)
---
> Ported from the Claude Code `hookify` plugin ([source](https://github.com/ste-bah/test-plugins-official/tree/main/plugins/hookify), Apache-2.0).

# Configure Hookify Rules

**First read the rule-writing reference** at `.archon/skills/hookify-writing-rules/SKILL.md` (project install; or the equivalent user-global skill root) to understand the rule format.

Enable or disable existing hookify rules interactively. Use only the Glob, Read, and Edit tools for this task.

## Steps

### 1. Find Existing Rules

Use the Glob tool to find all hookify rule files:
```
pattern: ".archon/hookify/*.local.md"
```

If no rules found, inform the user:
```
No hookify rules configured yet. Use `/hookify` to create your first rule.
```

### 2. Read Current State

For each rule file:
- Read the file
- Extract `name` and `enabled` fields from frontmatter
- Build a list of rules with their current state

### 3. Ask User Which Rules to Toggle

Ask the user in chat (plain text) which rules to enable or disable, listing every rule with its current state. The user may select several. For example:

```
Which rules would you like to enable or disable?

1. warn-dangerous-rm (currently enabled) - Warns about rm -rf commands
2. warn-console-log (currently disabled) - Warns about console.log in code
3. require-tests (currently enabled) - Requires tests before stopping
```

**Option format:**
- Label: `{rule-name} (currently {enabled|disabled})`
- Description: Brief description from the rule's message or pattern

### 4. Parse User Selection

For each selected rule:
- Determine current state from the label (enabled/disabled)
- Toggle state: enabled -> disabled, disabled -> enabled

### 5. Update Rule Files

For each rule to toggle:
- Use the Read tool to read the current content
- Use the Edit tool to change `enabled: true` to `enabled: false` (or vice versa)
- Handle both with and without quotes

**Edit pattern for enabling:**
```
old_string: "enabled: false"
new_string: "enabled: true"
```

**Edit pattern for disabling:**
```
old_string: "enabled: true"
new_string: "enabled: false"
```

### 6. Confirm Changes

Show the user what was changed:

```
## Hookify Rules Updated

**Enabled:**
- warn-console-log

**Disabled:**
- warn-dangerous-rm

**Unchanged:**
- require-tests

Changes apply immediately - no restart needed
```

## Important Notes

- Changes take effect immediately on next tool use
- You can also manually edit `.archon/hookify/*.local.md` files
- To permanently remove a rule, delete its `.local.md` file
- Use `/hookify-list` to see all configured rules

## Edge Cases

**No rules to configure:**
- Show a message about using `/hookify` to create rules first

**User selects no rules:**
- Inform that no changes were made

**File read/write errors:**
- Inform the user of the specific error
- Suggest manual editing as fallback
