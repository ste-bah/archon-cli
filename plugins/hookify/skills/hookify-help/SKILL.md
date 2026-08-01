---
name: hookify-help
description: Explain how the hookify plugin works, its rule format, and its skills; use when the user asks for help with hookify or how to create hook rules.
license-source: https://github.com/ste-bah/test-plugins-official/tree/main/plugins/hookify (Apache-2.0)
---
> Ported from the Claude Code `hookify` plugin ([source](https://github.com/ste-bah/test-plugins-official/tree/main/plugins/hookify), Apache-2.0).

# Hookify Plugin Help

Explain how the hookify plugin works and how to use it.

## Overview

The hookify plugin makes it easy to create custom hooks that prevent unwanted behaviors. Instead of editing hook configuration files by hand, users create simple markdown configuration files that define patterns to watch for.

## How It Works

### 1. Hook System

Hookify installs generic hooks that run on these events:
- **PreToolUse**: Before any tool executes (Bash, Edit, Write, etc.)
- **PostToolUse**: After a tool executes
- **Stop**: When the agent wants to stop working
- **UserPromptSubmit**: When the user submits a prompt

These hooks read configuration files from `.archon/hookify/*.local.md` and check if any rules match the current operation.

### 2. Configuration Files

Users create rules in `.archon/hookify/{rule-name}.local.md` files:

```markdown
---
name: warn-dangerous-rm
enabled: true
event: bash
pattern: rm\s+-rf
---

⚠️ **Dangerous rm command detected!**

This command could delete important files. Please verify the path.
```

**Key fields:**
- `name`: Unique identifier for the rule
- `enabled`: true/false to activate/deactivate
- `event`: bash, file, stop, prompt, or all
- `pattern`: Regex pattern to match

The message body is what the agent sees when the rule triggers.

### 3. Creating Rules

**Option A: Use the /hookify skill**
```
/hookify Don't use console.log in production files
```

This analyzes your request and creates the appropriate rule file.

**Option B: Create manually**
Create `.archon/hookify/my-rule.local.md` with the format above.

**Option C: Analyze conversation**
```
/hookify
```

Without arguments, hookify analyzes recent conversation to find behaviors you want to prevent.

## Available Skills

- **`/hookify`** - Create hooks from conversation analysis or explicit instructions
- **`/hookify-help`** - Show this help (what you're reading now)
- **`/hookify-list`** - List all configured hooks
- **`/hookify-configure`** - Enable/disable existing hooks interactively
- **`/hookify-writing-rules`** - Reference for rule file format and syntax

## Example Use Cases

**Prevent dangerous commands:**
```markdown
---
name: block-chmod-777
enabled: true
event: bash
pattern: chmod\s+777
---

Don't use chmod 777 - it's a security risk. Use specific permissions instead.
```

**Warn about debugging code:**
```markdown
---
name: warn-console-log
enabled: true
event: file
pattern: console\.log\(
---

Console.log detected. Remember to remove debug logging before committing.
```

**Require tests before stopping:**
```markdown
---
name: require-tests
enabled: true
event: stop
pattern: .*
---

Did you run tests before finishing? Make sure `npm test` or equivalent was executed.
```

## Pattern Syntax

Use Python regex syntax:
- `\s` - whitespace
- `\.` - literal dot
- `|` - OR
- `+` - one or more
- `*` - zero or more
- `\d` - digit
- `[abc]` - character class

**Examples:**
- `rm\s+-rf` - matches "rm -rf"
- `console\.log\(` - matches "console.log("
- `(eval|exec)\(` - matches "eval(" or "exec("
- `\.env$` - matches files ending in .env

## Important Notes

**No Restart Needed**: Hookify rules (`.local.md` files) take effect immediately on the next tool use. Once the hookify hooks are enabled in `.archon/settings.json`, they read your rules dynamically.

**Block or Warn**: Rules can either `block` operations (prevent execution) or `warn` (show message but allow). Set `action: block` or `action: warn` in the rule's frontmatter.

**Rule Files**: Keep rules in `.archon/hookify/*.local.md` - they should be git-ignored (add `.archon/hookify/*.local.md` to .gitignore if needed).

**Disable Rules**: Set `enabled: false` in frontmatter or delete the file.

## Troubleshooting

**Hook not triggering:**
- Check the rule file is in the `.archon/hookify/` directory
- Verify `enabled: true` in frontmatter
- Confirm the pattern is valid regex
- Test pattern: `python3 -c "import re; print(re.search('your_pattern', 'test_text'))"`
- Verify the hookify hooks are merged into `.archon/settings.json` (see the plugin README's "Enable hooks" section)
- Rules take effect immediately - no restart needed

**Import errors:**
- Check Python 3 is available: `python3 --version`
- Verify the hookify plugin scripts are installed at `.archon/plugins/hookify/scripts/`

**Pattern not matching:**
- Test regex separately
- Check for escaping issues (use unquoted patterns in YAML)
- Try simpler pattern first, then refine

## Getting Started

1. Create your first rule:
   ```
   /hookify Warn me when I try to use rm -rf
   ```

2. Try to trigger it:
   - Ask the agent to run `rm -rf /tmp/test`
   - You should see the warning

3. Refine the rule by editing `.archon/hookify/warn-rm.local.md`

4. Create more rules as you encounter unwanted behaviors

For more examples, check the `.archon/plugins/hookify/examples/` directory.
