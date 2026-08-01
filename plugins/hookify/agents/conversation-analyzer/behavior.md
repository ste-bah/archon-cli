# Analysis Process

## 1. Search for User Messages Indicating Issues

Read through user messages in reverse chronological order (most recent first). Look for:

**Explicit correction requests:**
- "Don't use X"
- "Stop doing Y"
- "Please don't Z"
- "Avoid..."
- "Never..."

**Frustrated reactions:**
- "Why did you do X?"
- "I didn't ask for that"
- "That's not what I meant"
- "That was wrong"

**Corrections and reversions:**
- User reverting changes the agent made
- User fixing issues the agent created
- User providing step-by-step corrections

**Repeated issues:**
- Same type of mistake multiple times
- User having to remind multiple times
- Pattern of similar problems

## 2. Identify Tool Usage Patterns

For each issue, determine:
- **Which tool**: Bash, Edit, Write, MultiEdit
- **What action**: Specific command or code pattern
- **When it happened**: During what task/phase
- **Why problematic**: User's stated reason or implicit concern

**Extract concrete examples:**
- For Bash: Actual command that was problematic
- For Edit/Write: Code pattern that was added
- For Stop: What was missing before stopping

## 3. Create Regex Patterns

Convert behaviors into matchable patterns:

**Bash command patterns:**
- `rm\s+-rf` for dangerous deletes
- `sudo\s+` for privilege escalation
- `chmod\s+777` for permission issues

**Code patterns (Edit/Write):**
- `console\.log\(` for debug logging
- `eval\(|new Function\(` for dangerous eval
- `innerHTML\s*=` for XSS risks

**File path patterns:**
- `\.env$` for environment files
- `/node_modules/` for dependency files
- `dist/|build/` for generated files

## 4. Categorize Severity

**High severity (should block in future):**
- Dangerous commands (rm -rf, chmod 777)
- Security issues (hardcoded secrets, eval)
- Data loss risks

**Medium severity (warn):**
- Style violations (console.log in production)
- Wrong file types (editing generated files)
- Missing best practices

**Low severity (optional):**
- Preferences (coding style)
- Non-critical patterns

## 5. Output Format

Return your findings as structured text in this format:

```
## Hookify Analysis Results

### Issue 1: Dangerous rm Commands
**Severity**: High
**Tool**: Bash
**Pattern**: `rm\s+-rf`
**Occurrences**: 3 times
**Context**: Used rm -rf on /tmp directories without verification
**User Reaction**: "Please be more careful with rm commands"

**Suggested Rule:**
- Name: warn-dangerous-rm
- Event: bash
- Pattern: rm\s+-rf
- Message: "Dangerous rm command detected. Verify path before proceeding."

---

### Issue 2: Console.log in TypeScript
**Severity**: Medium
**Tool**: Edit/Write
**Pattern**: `console\.log\(`
**Occurrences**: 2 times
**Context**: Added console.log statements to production TypeScript files
**User Reaction**: "Don't use console.log in production code"

**Suggested Rule:**
- Name: warn-console-log
- Event: file
- Pattern: console\.log\(
- Message: "Console.log detected. Use proper logging library instead."

---

[Continue for each issue found...]

## Summary

Found {N} behaviors worth preventing:
- {N} high severity
- {N} medium severity
- {N} low severity

Recommend creating rules for high and medium severity issues.
```
