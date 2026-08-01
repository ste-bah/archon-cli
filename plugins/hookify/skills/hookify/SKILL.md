---
name: hookify
description: Create hookify hook rules that prevent unwanted behaviors, from conversation analysis or explicit instructions; use when the user wants to block or be warned about specific commands, code patterns, or behaviors.
license-source: https://github.com/ste-bah/test-plugins-official/tree/main/plugins/hookify (Apache-2.0)
---
> Ported from the Claude Code `hookify` plugin ([source](https://github.com/ste-bah/test-plugins-official/tree/main/plugins/hookify), Apache-2.0).

# Hookify - Create Hooks from Unwanted Behaviors

**FIRST: Read the rule-writing reference** at `.archon/skills/hookify-writing-rules/SKILL.md` (project install; or the equivalent user-global skill root) to understand rule file format and syntax.

Create hook rules to prevent problematic behaviors by analyzing the conversation or from explicit user instructions.

## Your Task

You will help the user create hookify rules to prevent unwanted behaviors. Follow these steps:

### Step 1: Gather Behavior Information

Arguments, if any, are appended to the end of this prompt.

**If arguments are provided:**
- The user has given specific instructions in those arguments
- Still analyze recent conversation (last 10-15 user messages) for additional context
- Look for examples of the behavior happening

**If no arguments are provided:**
- Launch the conversation-analyzer agent to find problematic behaviors
- The agent will scan user prompts for frustration signals
- The agent will return structured findings

**To analyze conversation:**
Spawn a `hookify:conversation-analyzer` subagent with the Agent tool, with a prompt like:

```
You are analyzing an Archon conversation to find behaviors the user wants to prevent.

Read user messages in the current conversation and identify:
1. Explicit requests to avoid something ("don't do X", "stop doing Y")
2. Corrections or reversions (user fixing the agent's actions)
3. Frustrated reactions ("why did you do X?", "I didn't ask for that")
4. Repeated issues (same problem multiple times)

For each issue found, extract:
- What tool was used (Bash, Edit, Write, etc.)
- Specific pattern or command
- Why it was problematic
- User's stated reason

Return findings as a structured list with:
- category: Type of issue
- tool: Which tool was involved
- pattern: Regex or literal pattern to match
- context: What happened
- severity: high/medium/low

Focus on the most recent issues (last 20-30 messages). Don't go back further unless explicitly asked.
```

### Step 2: Present Findings to User

After gathering behaviors (from arguments or the agent), present them to the user and ask which ones to act on. Ask these questions in chat (plain text), listing the options clearly:

**Question 1: Which behaviors to hookify?**
- Heading: "Create Rules"
- The user may pick several
- Options: List each detected behavior (max 4)
  - Label: Short description (e.g., "Block rm -rf")
  - Description: Why it's problematic

**Question 2: For each selected behavior, ask about action:**
- "Should this block the operation or just warn?"
- Options:
  - "Just warn" (action: warn - shows message but allows)
  - "Block operation" (action: block - prevents execution)

**Question 3: Ask for example patterns:**
- "What patterns should trigger this rule?"
- Show detected patterns
- Allow the user to refine or add more

### Step 3: Generate Rule Files

For each confirmed behavior, create a `.archon/hookify/{rule-name}.local.md` file:

**Rule naming convention:**
- Use kebab-case
- Be descriptive: `block-dangerous-rm`, `warn-console-log`, `require-tests-before-stop`
- Start with action verb: block, warn, prevent, require

**File format:**
```markdown
---
name: {rule-name}
enabled: true
event: {bash|file|stop|prompt|all}
pattern: {regex pattern}
action: {warn|block}
---

{Message to show the agent when rule triggers}
```

**Action values:**
- `warn`: Show message but allow operation (default)
- `block`: Prevent operation or stop session

**For more complex rules (multiple conditions):**
```markdown
---
name: {rule-name}
enabled: true
event: file
conditions:
  - field: file_path
    operator: regex_match
    pattern: \.env$
  - field: new_text
    operator: contains
    pattern: API_KEY
---

{Warning message}
```

### Step 4: Create Files and Confirm

**IMPORTANT**: Rule files must be created in the current working directory's `.archon/hookify/` folder, NOT the plugin directory.

Use the current working directory (where Archon was started) as the base path.

1. Check if `.archon/hookify/` directory exists in the current working directory
   - If not, create it first with: `mkdir -p .archon/hookify`

2. Use the Write tool to create each `.archon/hookify/{name}.local.md` file
   - Use relative path from the current working directory: `.archon/hookify/{name}.local.md`
   - The path should resolve to the project's `.archon` directory, not the plugin's

3. Show the user what was created:
   ```
   Created 3 hookify rules:
   - .archon/hookify/dangerous-rm.local.md
   - .archon/hookify/console-log.local.md
   - .archon/hookify/sensitive-files.local.md

   These rules will trigger on:
   - dangerous-rm: Bash commands matching "rm -rf"
   - console-log: Edits adding console.log statements
   - sensitive-files: Edits to .env or credentials files
   ```

4. Verify files were created in the correct location by listing them

5. Inform the user: **"Rules are active immediately - no restart needed!"**

   The hookify hooks are already loaded and will read your new rules on the next tool use.
   (This assumes the hookify hooks are enabled in `.archon/settings.json` - see the plugin README's "Enable hooks" section if they are not.)

## Event Types Reference

- **bash**: Matches Bash tool commands
- **file**: Matches Edit, Write, MultiEdit tools
- **stop**: Matches when the agent wants to stop (use for completion checks)
- **prompt**: Matches when the user submits prompts
- **all**: Matches all events

## Pattern Writing Tips

**Bash patterns:**
- Match dangerous commands: `rm\s+-rf|chmod\s+777|dd\s+if=`
- Match specific tools: `npm\s+install\s+|pip\s+install`

**File patterns:**
- Match code patterns: `console\.log\(|eval\(|innerHTML\s*=`
- Match file paths: `\.env$|\.git/|node_modules/`

**Stop patterns:**
- Check for missing steps: (check transcript or completion criteria)

## Example Workflow

**User says**: "/hookify Don't use rm -rf without asking me first"

**Your response**:
1. Analyze: User wants to prevent rm -rf commands
2. Ask: "Should I block this command or just warn you?"
3. User selects: "Just warn"
4. Create `.archon/hookify/dangerous-rm.local.md`:
   ```markdown
   ---
   name: warn-dangerous-rm
   enabled: true
   event: bash
   pattern: rm\s+-rf
   ---

   ⚠️ **Dangerous rm command detected**

   You requested to be warned before using rm -rf.
   Please verify the path is correct.
   ```
5. Confirm: "Created hookify rule. It's active immediately - try triggering it!"

## Important Notes

- **No restart needed**: Rules take effect immediately on the next tool use
- **File location**: Create files in the project's `.archon/hookify/` directory (current working directory), NOT the plugin's directory
- **Regex syntax**: Use Python regex syntax (raw strings, no need to escape in YAML)
- **Action types**: Rules can `warn` (default) or `block` operations
- **Testing**: Test rules immediately after creating them

## Troubleshooting

**If rule file creation fails:**
1. Check current working directory with pwd
2. Ensure `.archon/hookify/` directory exists (create with mkdir if needed)
3. Use absolute path if needed: `{cwd}/.archon/hookify/{name}.local.md`
4. Verify file was created with Glob or ls

**If rule doesn't trigger after creation:**
1. Verify the file is in the project's `.archon/hookify/`, not the plugin's directory
2. Check the file with the Read tool to ensure the pattern is correct
3. Test the pattern with: `python3 -c "import re; print(re.search(r'pattern', 'test text'))"`
4. Verify `enabled: true` in frontmatter
5. Verify the hookify hooks are merged into `.archon/settings.json` (see the plugin README)
6. Remember: Rules work immediately, no restart needed

**If blocking seems too strict:**
1. Change `action: block` to `action: warn` in the rule file
2. Or adjust the pattern to be more specific
3. Changes take effect on next tool use

Use TodoWrite to track your progress through the steps.
