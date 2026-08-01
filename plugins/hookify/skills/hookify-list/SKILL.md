---
name: hookify-list
description: List all configured hookify rules in the project with their status; use when the user asks what hookify rules exist or whether a rule is enabled.
license-source: https://github.com/ste-bah/test-plugins-official/tree/main/plugins/hookify (Apache-2.0)
---
> Ported from the Claude Code `hookify` plugin ([source](https://github.com/ste-bah/test-plugins-official/tree/main/plugins/hookify), Apache-2.0).

# List Hookify Rules

**First read the rule-writing reference** at `.archon/skills/hookify-writing-rules/SKILL.md` (project install; or the equivalent user-global skill root) to understand the rule format.

Show all configured hookify rules in the project. Use only the Glob and Read tools for this task.

## Steps

1. Use the Glob tool to find all hookify rule files:
   ```
   pattern: ".archon/hookify/*.local.md"
   ```

2. For each file found:
   - Use the Read tool to read the file
   - Extract frontmatter fields: name, enabled, event, pattern
   - Extract message preview (first 100 chars)

3. Present results in a table:

```
## Configured Hookify Rules

| Name | Enabled | Event | Pattern | File |
|------|---------|-------|---------|------|
| warn-dangerous-rm | ✅ Yes | bash | rm\s+-rf | dangerous-rm.local.md |
| warn-console-log | ✅ Yes | file | console\.log\( | console-log.local.md |
| check-tests | ❌ No | stop | .* | require-tests.local.md |

**Total**: 3 rules (2 enabled, 1 disabled)
```

4. For each rule, show a brief preview:
```
### warn-dangerous-rm
**Event**: bash
**Pattern**: `rm\s+-rf`
**Message**: "⚠️ **Dangerous rm command detected!** This command could delete..."

**Status**: ✅ Active
**File**: .archon/hookify/dangerous-rm.local.md
```

5. Add helpful footer:
```
---

To modify a rule: Edit the .local.md file directly
To disable a rule: Set `enabled: false` in frontmatter
To enable a rule: Set `enabled: true` in frontmatter
To delete a rule: Remove the .local.md file
To create a rule: Use the `/hookify` skill

**Remember**: Changes take effect immediately - no restart needed
```

## If No Rules Found

If no hookify rules exist:

```
## No Hookify Rules Configured

You haven't created any hookify rules yet.

To get started:
1. Use `/hookify` to analyze conversation and create rules
2. Or manually create `.archon/hookify/my-rule.local.md` files
3. See `/hookify-help` for documentation

Example:
/hookify Warn me when I use console.log

Check `.archon/plugins/hookify/examples/` for example rule files.
```
