# Hookify Plugin

> Ported from the Claude Code `hookify` plugin ([source](https://github.com/ste-bah/test-plugins-official/tree/main/plugins/hookify), Apache-2.0).

Easily create custom hooks to prevent unwanted behaviors by analyzing conversation patterns or from explicit instructions.

## Overview

The hookify plugin makes it simple to create hooks without editing complex hook configuration files. Instead, you create lightweight markdown configuration files that define patterns to watch for and messages to show when those patterns match.

**Key features:**
- 🎯 Analyze conversations to find unwanted behaviors automatically
- 📝 Simple markdown configuration files with YAML frontmatter
- 🔍 Regex pattern matching for powerful rules
- 🚀 No coding required - just describe the behavior
- 🔄 Easy enable/disable without restarting

## Installation

Project-local install:

1. Copy `agents/` to `<project>/.archon/plugins/hookify/agents/`.
2. Copy each `skills/<skill>/` dir to `<project>/.archon/skills/<skill>/` (skills: `hookify`, `hookify-configure`, `hookify-list`, `hookify-help`, `hookify-writing-rules`).
3. Copy `scripts/` to `<project>/.archon/plugins/hookify/scripts/` and copy `examples/` to `<project>/.archon/plugins/hookify/examples/`.
4. Enable the hooks (see below).

Or user-global: agents to `~/.archon/plugins/hookify/agents/`, skills to `~/.config/archon/skills/` (or platform data dir + `archon/skills/`).

Or run `plugins/install.ps1 hookify` / `plugins/install.sh hookify` from the archon-cli repo root.

Restart archon after installing (skills and agents load at startup).

## Enable hooks

Hooks are NOT auto-loaded from plugin directories. To enable the hookify hooks:

1. Copy `scripts/` to `<project>/.archon/plugins/hookify/scripts/` (done above).
2. Merge `hooks/settings.snippet.json` into `<project>/.archon/settings.json` (create the file with that content if it doesn't exist; otherwise merge the `"hooks"` keys).

The snippet registers the four hookify hook executors (PreToolUse, PostToolUse, Stop, UserPromptSubmit) with `blocking: true` on PreToolUse and Stop so `action: block` rules can cancel operations via exit code 2.

If you install user-globally, change the script paths in the snippet from `.archon/plugins/hookify/scripts/...` to `~/.archon/plugins/hookify/scripts/...`.

On Windows, if `python3` resolves to the Microsoft Store stub, change `python3` to `python` in the merged settings.

## Quick Start

### 1. Create Your First Rule

```bash
/hookify Warn me when I use rm -rf commands
```

This analyzes your request and creates `.archon/hookify/warn-rm.local.md`.

### 2. Test It Immediately

**No restart needed!** Rules take effect on the very next tool use.

Ask the agent to run a command that should trigger the rule:
```
Run rm -rf /tmp/test
```

You should see the warning message immediately!

## Usage

### Main Skill: /hookify

**With arguments:**
```
/hookify Don't use console.log in TypeScript files
```
Creates a rule from your explicit instructions.

**Without arguments:**
```
/hookify
```
Analyzes recent conversation to find behaviors you've corrected or been frustrated by.

### Helper Skills

**List all rules:**
```
/hookify-list
```

**Configure rules interactively:**
```
/hookify-configure
```
Enable/disable existing rules through an interactive interface.

**Get help:**
```
/hookify-help
```

**Rule-format reference:**
```
/hookify-writing-rules
```

## Rule Configuration Format

### Simple Rule (Single Pattern)

`.archon/hookify/dangerous-rm.local.md`:
```markdown
---
name: block-dangerous-rm
enabled: true
event: bash
pattern: rm\s+-rf
action: block
---

⚠️ **Dangerous rm command detected!**

This command could delete important files. Please:
- Verify the path is correct
- Consider using a safer approach
- Make sure you have backups
```

**Action field:**
- `warn`: Shows warning but allows operation (default)
- `block`: Prevents operation from executing (PreToolUse) or stops session exit (Stop events)

### Advanced Rule (Multiple Conditions)

`.archon/hookify/sensitive-files.local.md`:
```markdown
---
name: warn-sensitive-files
enabled: true
event: file
action: warn
conditions:
  - field: file_path
    operator: regex_match
    pattern: \.env$|credentials|secrets
  - field: new_text
    operator: contains
    pattern: KEY
---

🔐 **Sensitive file edit detected!**

Ensure credentials are not hardcoded and file is in .gitignore.
```

**All conditions must match** for the rule to trigger.

## Event Types

- **`bash`**: Triggers on Bash tool commands
- **`file`**: Triggers on Edit, Write, MultiEdit tools
- **`stop`**: Triggers when the agent wants to stop (for completion checks)
- **`prompt`**: Triggers on user prompt submission
- **`all`**: Triggers on all events

## Pattern Syntax

Use Python regex syntax:

| Pattern | Matches | Example |
|---------|---------|---------|
| `rm\s+-rf` | rm -rf | rm -rf /tmp |
| `console\.log\(` | console.log( | console.log("test") |
| `(eval\|exec)\(` | eval( or exec( | eval("code") |
| `\.env$` | files ending in .env | .env, .env.local |
| `chmod\s+777` | chmod 777 | chmod 777 file.txt |

**Tips:**
- Use `\s` for whitespace
- Escape special chars: `\.` for literal dot
- Use `|` for OR: `(foo|bar)`
- Use `.*` to match anything
- Set `action: block` for dangerous operations
- Set `action: warn` (or omit) for informational warnings

## Examples

### Example 1: Block Dangerous Commands

```markdown
---
name: block-destructive-ops
enabled: true
event: bash
pattern: rm\s+-rf|dd\s+if=|mkfs|format
action: block
---

🛑 **Destructive operation detected!**

This command can cause data loss. Operation blocked for safety.
Please verify the exact path and use a safer approach.
```

**This rule blocks the operation** - the agent will not be allowed to execute these commands.

### Example 2: Warn About Debug Code

```markdown
---
name: warn-debug-code
enabled: true
event: file
pattern: console\.log\(|debugger;|print\(
action: warn
---

🐛 **Debug code detected**

Remember to remove debugging statements before committing.
```

**This rule warns but allows** - the agent sees the message but can still proceed.

### Example 3: Require Tests Before Stopping

```markdown
---
name: require-tests-run
enabled: false
event: stop
action: block
conditions:
  - field: transcript
    operator: not_contains
    pattern: npm test|pytest|cargo test
---

**Tests not detected in transcript!**

Before stopping, please run tests to verify your changes work correctly.
```

**This blocks the agent from stopping** if no test commands appear in the session transcript. Enable only when you want strict enforcement. (Requires the Stop event to provide a `transcript_path` - see "Differences from the Claude plugin".)

## Advanced Usage

### Multiple Conditions

Check multiple fields simultaneously:

```markdown
---
name: api-key-in-typescript
enabled: true
event: file
conditions:
  - field: file_path
    operator: regex_match
    pattern: \.tsx?$
  - field: new_text
    operator: regex_match
    pattern: (API_KEY|SECRET|TOKEN)\s*=\s*["']
---

🔐 **Hardcoded credential in TypeScript!**

Use environment variables instead of hardcoded values.
```

### Operators Reference

- `regex_match`: Pattern must match (most common)
- `contains`: String must contain pattern
- `equals`: Exact string match
- `not_contains`: String must NOT contain pattern
- `starts_with`: String starts with pattern
- `ends_with`: String ends with pattern

### Field Reference

**For bash events:**
- `command`: The bash command string

**For file events:**
- `file_path`: Path to file being edited
- `new_text`: New content being added (Edit, Write)
- `old_text`: Old content being replaced (Edit only)
- `content`: File content (Write only)

**For prompt events:**
- `user_prompt`: The user's submitted prompt text

**For stop events:**
- Use general matching on session state

## Management

### Enable/Disable Rules

**Temporarily disable:**
Edit the `.local.md` file and set `enabled: false`

**Re-enable:**
Set `enabled: true`

**Or use the interactive skill:**
```
/hookify-configure
```

### Delete Rules

Simply delete the `.local.md` file:
```bash
rm .archon/hookify/my-rule.local.md
```

### View All Rules

```
/hookify-list
```

## Requirements

- Python 3.7+
- No external dependencies (uses stdlib only)

## Troubleshooting

**Rule not triggering:**
1. Check the rule file exists in the `.archon/hookify/` directory (in the project root, not the plugin directory)
2. Verify `enabled: true` in frontmatter
3. Test the regex pattern separately
4. Verify the hooks snippet is merged into `.archon/settings.json` (see "Enable hooks")
5. Rules should work immediately - no restart needed
6. Try `/hookify-list` to see if the rule is loaded

**Import errors:**
- Ensure Python 3 is available: `python3 --version`
- Check the hookify scripts are installed at `.archon/plugins/hookify/scripts/`

**Pattern not matching:**
- Test regex: `python3 -c "import re; print(re.search(r'pattern', 'text'))"`
- Use unquoted patterns in YAML to avoid escaping issues
- Start simple, then add complexity

**Hook seems slow:**
- Keep patterns simple (avoid complex regex)
- Use specific event types (bash, file) instead of "all"
- Limit number of active rules

## Differences from the Claude plugin

- **Rule file location**: rules moved from `.claude/hookify.<rule>.local.md` to `.archon/hookify/<rule>.local.md` (a subdirectory instead of a filename prefix). All skills, hook scripts, and examples use the new location.
- **Hook output protocol**: Claude's hooks returned JSON (`systemMessage`, `hookSpecificOutput.permissionDecision: "deny"`, `decision: "block"`). Archon does not interpret hook stdout JSON; the port prints warn messages to stdout, prints block messages to stderr, and uses exit code 2 with `blocking: true` (in the settings snippet) to cancel PreToolUse tool calls and Stop events. PostToolUse and UserPromptSubmit rules are informational only. How prominently non-blocking hook output is surfaced depends on the Archon version.
- **Hook registration**: Claude auto-loaded `hooks/hooks.json` from the plugin; Archon requires merging `hooks/settings.snippet.json` into `.archon/settings.json` (see "Enable hooks").
- **Script layout**: the source's `hooks/*.py` + `core/*.py` package (with empty `core/matchers/utils` `__init__.py` files and plugin-root-env-based imports) was flattened into a single `scripts/` directory with sibling imports; behavior is unchanged.
- **Event JSON fields**: scripts read Archon's event shape (`tool_args`, `event`) instead of Claude's (`tool_input`, `hook_event_name`), with a defensive fallback to the Claude names.
- **Stop `transcript` conditions**: Claude's Stop event always provided `transcript_path`; Archon's documented Stop event does not list it. If absent, `transcript` conditions never match and such rules stay inert (they fail open, never blocking by accident).
- **`prompt` events**: the user-prompt text field is looked up defensively (`user_prompt`, then `prompt`) since the Archon field name is not specified in the hooks doc.
- **Interactive questions**: Claude's `AskUserQuestion` tool has no Archon equivalent; the `hookify` and `hookify-configure` skills ask the same questions in plain chat instead.
- **Skill naming**: commands `/hookify:list`, `/hookify:configure`, `/hookify:help` became skills `/hookify-list`, `/hookify-configure`, `/hookify-help`; the `writing-rules` skill became `/hookify-writing-rules`. Skills load a reference file from `.archon/skills/hookify-writing-rules/SKILL.md` instead of using Claude's Skill tool.
- **Marketplace install**: the "Claude Code Marketplace" install section was replaced with Archon copy-install instructions.
- **Source `.gitignore`**: not ported (repo housekeeping); add `.archon/hookify/*.local.md` to your own `.gitignore`.
- **License note**: the source README said "MIT License", but the bundled LICENSE file is Apache-2.0; the LICENSE file is copied verbatim and treated as authoritative.

## Contributing

Found a useful rule pattern? Consider sharing example files via PR!

## Future Enhancements

- Severity levels (error/warning/info distinctions)
- Rule templates library
- Interactive pattern builder
- Hook testing utilities
- JSON format support (in addition to markdown)

## License

Apache License 2.0 (see LICENSE).
